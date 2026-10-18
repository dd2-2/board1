import { useEffect, useState, type ReactNode } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import summaryPrompt from './assets/prompts/summary.txt?raw'
import StepCard from './StepCard'

interface SummaryPptResult {
  outputPath: string
  slideCount: number
  warnings: string[]
}

interface RunResult {
  txtPath: string
  summaryTxtPath?: string
  result?: SummaryPptResult
  error?: string
}

function fileName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

function leadingNumber(path: string): number | null {
  const m = fileName(path).match(/\d+/)
  return m ? parseInt(m[0], 10) : null
}

/** 디자인 템플릿(3페이지) PPTX 하나 + 요약 TXT 여러 개 → TXT마다 "요약완성_{TXT이름}.pptx"를 TXT 옆에 만든다.
 *  GPT 자동 모드에선 TXT가 원본 녹취록이고, GPT가 요약 TXT(AI요약_)를 먼저 만든 뒤 그걸로 PPT를 만든다. */
export default function SummaryTab({ ai, apiCard }: { ai: boolean; apiCard: ReactNode }) {
  const [templatePath, setTemplatePath] = useState<string | null>(null)
  // 저장 폴더 (null이면 TXT 옆)
  const [outputDir, setOutputDir] = useState<string | null>(null)
  const [txtPaths, setTxtPaths] = useState<string[]>([])
  const [storyboards, setStoryboards] = useState<string[]>([])
  // 녹취 TXT → 스토리보드 PPTX (GPT 자동 모드 전용, ''이면 스토리보드 없이)
  const [storyboardFor, setStoryboardFor] = useState<Record<string, string>>({})
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState('')
  const [results, setResults] = useState<RunResult[] | null>(null)
  const [copied, setCopied] = useState(false)
  const [dragActive, setDragActive] = useState(false)

  function addTxt(paths: string[]) {
    setTxtPaths((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))])
    setResults(null)
  }

  // 새로 들어온 녹취/스토리보드는 파일명 첫 숫자로 자동 매칭 (각 1개뿐이면 그냥 짝지음)
  useEffect(() => {
    setStoryboardFor((prev) => {
      const next: Record<string, string> = {}
      for (const t of txtPaths) {
        if (t in prev && (prev[t] === '' || storyboards.includes(prev[t]))) {
          next[t] = prev[t]
          continue
        }
        const n = leadingNumber(t)
        const match =
          (n !== null && storyboards.find((s) => leadingNumber(s) === n)) ||
          (txtPaths.length === 1 && storyboards.length === 1 ? storyboards[0] : '')
        next[t] = match || ''
      }
      return next
    })
  }, [txtPaths, storyboards])

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over') {
        setDragActive(true)
        return
      }
      setDragActive(false)
      if (event.payload.type !== 'drop') return
      const pptx = event.payload.paths.filter((p) => p.toLowerCase().endsWith('.pptx'))
      const txt = event.payload.paths.filter((p) => /\.(txt|hwp|hwpx)$/i.test(p))
      if (pptx.length > 0) {
        setTemplatePath(pptx[pptx.length - 1])
        setResults(null)
      }
      if (txt.length > 0) addTxt(txt)
    })
    return () => {
      unlisten.then((f) => f())
    }
  }, [])

  async function pickTemplate() {
    const selected = await open({ multiple: false, filters: [{ name: 'PowerPoint', extensions: ['pptx'] }] })
    if (typeof selected === 'string') {
      setTemplatePath(selected)
      setResults(null)
    }
  }

  async function pickTxt() {
    const selected = await open({ multiple: true, filters: [{ name: 'TXT / HWP / HWPX', extensions: ['txt', 'hwp', 'hwpx'] }] })
    if (Array.isArray(selected)) addTxt(selected)
    else if (typeof selected === 'string') addTxt([selected])
  }

  async function pickStoryboards() {
    const selected = await open({ multiple: true, filters: [{ name: 'PowerPoint', extensions: ['pptx'] }] })
    const list = Array.isArray(selected) ? selected : typeof selected === 'string' ? [selected] : []
    setStoryboards((prev) => [...prev, ...list.filter((p) => !prev.includes(p))])
  }

  async function pickOutputDir() {
    const selected = await open({ directory: true, multiple: false })
    if (typeof selected === 'string') setOutputDir(selected)
  }

  async function copyPrompt() {
    await writeText(summaryPrompt)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  async function run() {
    if (!templatePath || txtPaths.length === 0) return
    setRunning(true)
    const out: RunResult[] = []
    for (const [i, txtPath] of txtPaths.entries()) {
      let summaryTxtPath: string | undefined
      try {
        if (ai) {
          setProgress(`GPT 요약 생성 중... (${i + 1}/${txtPaths.length})`)
          summaryTxtPath = await invoke<string>('ai_generate_summary', {
            transcriptPath: txtPath,
            storyboardPath: storyboardFor[txtPath] || null,
          })
        }
        const result = await invoke<SummaryPptResult>('build_summary_ppt', {
          templatePath,
          txtPath: summaryTxtPath ?? txtPath,
          outputDir,
        })
        out.push({ txtPath, summaryTxtPath, result })
      } catch (e) {
        out.push({ txtPath, summaryTxtPath, error: String(e) })
      }
      setResults([...out])
    }
    setProgress('')
    setRunning(false)
  }

  return (
    <>
      <div className="steps">
        {ai ? (
          apiCard
        ) : (
          <>
            <StepCard num={1} title="프롬프트 복사">
              <button onClick={copyPrompt}>{copied ? '복사됨' : '요약 프롬프트'}</button>
            </StepCard>
            <StepCard
              num={2}
              title="ChatGPT로 요약"
              desc={
                <>
                  <ol className="step-list">
                    <li>스토리보드.pptx</li>
                    <li>강의 녹취.txt or hwp</li>
                  </ol>
                  + 복사된 프롬프트 → GPT
                </>
              }
            />
          </>
        )}
        <StepCard num={ai ? 2 : 3} title="파일 선택" desc={
            ai ? (
              '템플릿 + 녹취 (+스토리보드)'
            ) : (
              <ol className="step-list">
                <li>템플릿 (다운로드교안 디자인 파일)</li>
                <li>GPT에서 나온 요약 파일</li>
              </ol>
            )
          } active={dragActive}>
          <button className="file-btn" onClick={pickTemplate} title={templatePath ?? ''}>
            {templatePath ? `템플릿: ${fileName(templatePath)}` : '템플릿 선택'}
          </button>
          <button className="file-btn" onClick={pickTxt}>{ai ? '녹취 추가' : 'TXT 추가'}</button>
          {ai && <button className="file-btn" onClick={pickStoryboards}>스토리보드 추가</button>}
        </StepCard>
        <StepCard
          num={ai ? 3 : 4}
          title="실행"
          desc={`→ ${outputDir ? fileName(outputDir) + '/' : 'TXT 옆 '}요약완성_이름.pptx`}
        >
          <button className="secondary" onClick={pickOutputDir} title={outputDir ?? 'TXT와 같은 폴더에 저장'}>
            {outputDir ? `경로: ${fileName(outputDir)}` : '경로 지정'}
          </button>
          <button className="primary" onClick={run} disabled={!templatePath || txtPaths.length === 0 || running}>
            {running
              ? progress || '생성 중...'
              : `${ai ? 'GPT로 ' : ''}PPT 만들기${txtPaths.length > 1 ? ` (${txtPaths.length}건)` : ''}`}
          </button>
        </StepCard>
      </div>

      {txtPaths.length > 0 && (
        <div className="pair-list">
          {txtPaths.map((t) => (
            <div className="pair-row" key={t}>
              <div className="pair-pptx" title={t}>
                {fileName(t)}
              </div>
              {ai && (
                <>
                  <span className="pair-arrow">+</span>
                  <select
                    className="pair-select"
                    value={storyboardFor[t] ?? ''}
                    onChange={(e) => setStoryboardFor((prev) => ({ ...prev, [t]: e.target.value }))}
                  >
                    <option value="">(스토리보드 없음)</option>
                    {storyboards.map((sb) => (
                      <option key={sb} value={sb}>
                        {fileName(sb)}
                      </option>
                    ))}
                  </select>
                </>
              )}
              <button
                className="pair-remove"
                onClick={() => setTxtPaths((prev) => prev.filter((p) => p !== t))}
                title="목록에서 제거"
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

      {results && (
        <div className="result">
          {results.map((r) => (
            <div className="batch-result" key={r.txtPath}>
              <h3 className="batch-result-title">{fileName(r.txtPath)}</h3>
              {r.summaryTxtPath && <div className="batch-result-note">GPT 요약: {fileName(r.summaryTxtPath)}</div>}
              {r.result && (
                <div className="batch-result-saved">
                  → 저장됨: {fileName(r.result.outputPath)} (총 {r.result.slideCount}페이지)
                </div>
              )}
              {r.result?.warnings.map((w) => (
                <div className="warn-box" key={w}>
                  {w}
                </div>
              ))}
              {r.error && <div className="error-box">{r.error}</div>}
            </div>
          ))}
        </div>
      )}

    </>
  )
}
