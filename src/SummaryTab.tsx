import { useState, type ReactNode } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import summaryPrompt from './assets/prompts/summary.txt?raw'
import StepCard from './StepCard'
import { useDropTargets } from './useDropTargets'
import { useToast } from './toast'

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
export default function SummaryTab({
  ai,
  apiCard,
  resetButton,
}: {
  ai: boolean
  apiCard: ReactNode
  resetButton: ReactNode
}) {
  const [templatePath, setTemplatePath] = useState<string | null>(null)
  // 저장 폴더 (null이면 TXT 옆)
  const [outputDir, setOutputDir] = useState<string | null>(null)
  const [txtPaths, setTxtPaths] = useState<string[]>([])
  const [storyboards, setStoryboards] = useState<string[]>([])
  // 사용자가 드롭다운으로 직접 고른 스토리보드 (''이면 '없음'을 고른 것). 안 고른 녹취는 파일명 숫자로 자동 매칭
  const [manualStoryboard, setManualStoryboard] = useState<Record<string, string>>({})
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState('')
  const notify = useToast()
  const [results, setResults] = useState<RunResult[] | null>(null)
  const [copied, setCopied] = useState(false)

  function addTxt(paths: string[]) {
    setTxtPaths((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))])
    setResults(null)
  }

  function addStoryboards(paths: string[]) {
    setStoryboards((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))])
  }

  function setTemplate(path: string) {
    setTemplatePath(path)
    setResults(null)
  }

  /** 녹취에 짝지을 스토리보드: 직접 고른 값 → 파일명 첫 숫자가 같은 것 → (녹취·스토리보드 각 1개면) 그것 */
  function storyboardOf(t: string): string {
    if (t in manualStoryboard && (manualStoryboard[t] === '' || storyboards.includes(manualStoryboard[t]))) {
      return manualStoryboard[t]
    }
    const n = leadingNumber(t)
    const byNumber = n !== null ? storyboards.find((s) => leadingNumber(s) === n) : undefined
    return byNumber ?? (txtPaths.length === 1 && storyboards.length === 1 ? storyboards[0] : '')
  }

  // 각 버튼 위에 놓은 파일만, 그 버튼이 받는 형식만
  const dropHover = useDropTargets({
    template: { exts: ['pptx'], onDrop: (p) => setTemplate(p[p.length - 1]) },
    txt: { exts: ['txt', 'hwp', 'hwpx'], onDrop: addTxt },
    storyboard: { exts: ['pptx'], onDrop: addStoryboards },
  })
  const dropCls = (key: string) => `file-btn ${dropHover === key ? 'drop-hover' : ''}`

  async function pickTemplate() {
    const selected = await open({ multiple: false, filters: [{ name: 'PowerPoint', extensions: ['pptx'] }] })
    if (typeof selected === 'string') setTemplate(selected)
  }

  async function pickTxt() {
    const selected = await open({ multiple: true, filters: [{ name: 'TXT / HWP / HWPX', extensions: ['txt', 'hwp', 'hwpx'] }] })
    if (Array.isArray(selected)) addTxt(selected)
    else if (typeof selected === 'string') addTxt([selected])
  }

  async function pickStoryboards() {
    const selected = await open({ multiple: true, filters: [{ name: 'PowerPoint', extensions: ['pptx'] }] })
    addStoryboards(Array.isArray(selected) ? selected : typeof selected === 'string' ? [selected] : [])
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
            storyboardPath: storyboardOf(txtPath) || null,
          })
        }
        const result = await invoke<SummaryPptResult>('build_summary_ppt', {
          templatePath,
          txtPath: summaryTxtPath ?? txtPath,
          outputDir,
        })
        // 완료는 토스트로만 알리고, 실패만 아래 목록에 남김
        notify(
          `✓ ${fileName(txtPath)} → ${fileName(result.outputPath)} · ${result.slideCount}p${result.warnings.length > 0 ? ` · 경고 ${result.warnings.length}` : ''}`,
          result.warnings.length > 0 ? 'warn' : 'ok',
          [summaryTxtPath && `GPT 요약: ${summaryTxtPath}`, result.outputPath, ...result.warnings].filter(Boolean).join('\n'),
        )
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
        <StepCard num={ai ? 2 : 3} title="파일 선택" action={resetButton} desc={
            ai ? (
              '템플릿 + 녹취 (+스토리보드)'
            ) : (
              <ol className="step-list">
                <li>템플릿 (다운로드교안 디자인 파일)</li>
                <li>GPT에서 나온 요약 파일</li>
              </ol>
            )
          }>
          <button className={dropCls('template')} data-drop="template" onClick={pickTemplate} title={templatePath ?? ''}>
            {templatePath ? `템플릿: ${fileName(templatePath)}` : '템플릿 선택'}
          </button>
          <button className={dropCls('txt')} data-drop="txt" onClick={pickTxt}>{ai ? '녹취 추가' : 'TXT 추가'}</button>
          {ai && (
            <button className={dropCls('storyboard')} data-drop="storyboard" onClick={pickStoryboards}>
              {storyboards.length > 0 ? `스토리보드 추가 (${storyboards.length})` : '스토리보드 추가'}
            </button>
          )}
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
                    value={storyboardOf(t)}
                    onChange={(e) => setManualStoryboard((prev) => ({ ...prev, [t]: e.target.value }))}
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
        <div className="result-lines">
          {results.map((r) => {
            const warnings = r.result?.warnings ?? []
            return (
              <div
                className={`result-line ${r.error ? 'fail' : warnings.length > 0 ? 'warn' : 'ok'}`}
                key={r.txtPath}
                title={[r.summaryTxtPath && `GPT 요약: ${r.summaryTxtPath}`, r.result?.outputPath, r.error, ...warnings]
                  .filter(Boolean)
                  .join('\n')}
              >
                <span className="result-mark">{r.error ? '✕' : '✓'}</span>
                <span className="result-name">{fileName(r.txtPath)}</span>
                <span className="result-detail">
                  {r.error
                    ? r.error.split('\n')[0]
                    : `→ ${fileName(r.result!.outputPath)} · ${r.result!.slideCount}p${warnings.length > 0 ? ` · 경고 ${warnings.length}` : ''}`}
                </span>
              </div>
            )
          })}
        </div>
      )}

    </>
  )
}
