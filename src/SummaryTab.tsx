import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import summaryPrompt from './assets/prompts/summary.txt?raw'

interface SummaryPptResult {
  outputPath: string
  slideCount: number
}

interface RunResult {
  txtPath: string
  result?: SummaryPptResult
  error?: string
}

function fileName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/** 디자인 템플릿(3페이지) PPTX 하나 + 요약 TXT 여러 개 → TXT마다 "요약완성_{TXT이름}.pptx"를 TXT 옆에 만든다. */
export default function SummaryTab() {
  const [templatePath, setTemplatePath] = useState<string | null>(null)
  const [txtPaths, setTxtPaths] = useState<string[]>([])
  const [running, setRunning] = useState(false)
  const [results, setResults] = useState<RunResult[] | null>(null)
  const [copied, setCopied] = useState(false)
  const [dragActive, setDragActive] = useState(false)

  function addTxt(paths: string[]) {
    setTxtPaths((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))])
    setResults(null)
  }

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over') {
        setDragActive(true)
        return
      }
      setDragActive(false)
      if (event.payload.type !== 'drop') return
      const pptx = event.payload.paths.filter((p) => p.toLowerCase().endsWith('.pptx'))
      const txt = event.payload.paths.filter((p) => p.toLowerCase().endsWith('.txt'))
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
    const selected = await open({ multiple: true, filters: [{ name: 'Text', extensions: ['txt'] }] })
    if (Array.isArray(selected)) addTxt(selected)
    else if (typeof selected === 'string') addTxt([selected])
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
    for (const txtPath of txtPaths) {
      try {
        const result = await invoke<SummaryPptResult>('build_summary_ppt', { templatePath, txtPath })
        out.push({ txtPath, result })
      } catch (e) {
        out.push({ txtPath, error: String(e) })
      }
      setResults([...out])
    }
    setRunning(false)
  }

  return (
    <>
      <div className="prompt-section">
        <h2>GPT 프롬프트 (스토리보드+녹취 → 요약 TXT 생성용)</h2>
        <div className="prompt-card">
          <span>강의 요약 TXT 형식</span>
          <button onClick={copyPrompt}>{copied ? '복사됨' : '복사'}</button>
        </div>
      </div>

      <div className="field">
        <label>디자인 템플릿 PPTX (3페이지: 표지 / 학습목표·학습내용 / 본문 디자인)</label>
        <div className="picker-row">
          <div className={`picker-path ${templatePath ? '' : 'empty'}`} title={templatePath ?? ''}>
            {templatePath ? fileName(templatePath) : '선택 안 됨'}
          </div>
          <button onClick={pickTemplate}>템플릿 선택</button>
        </div>
      </div>

      <div className="field">
        <label>요약 TXT (여러 건 가능 — TXT마다 완성본 PPT가 TXT 옆에 저장됨)</label>
        <div className="picker-row">
          <button onClick={pickTxt}>TXT 추가</button>
        </div>
        {txtPaths.length > 0 && (
          <div className="pair-list">
            {txtPaths.map((t) => (
              <div className="pair-row" key={t}>
                <div className="pair-pptx" title={t}>
                  {fileName(t)}
                </div>
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
      </div>

      <div className="run-row">
        <button className="primary" onClick={run} disabled={!templatePath || txtPaths.length === 0 || running}>
          {running ? '생성 중...' : `PPT 만들기${txtPaths.length > 1 ? ` (${txtPaths.length}건)` : ''}`}
        </button>
      </div>

      {results && (
        <div className="result">
          {results.map((r) => (
            <div className="batch-result" key={r.txtPath}>
              <h3 className="batch-result-title">{fileName(r.txtPath)}</h3>
              {r.result && (
                <div className="batch-result-saved">
                  → 저장됨: {fileName(r.result.outputPath)} (총 {r.result.slideCount}페이지)
                </div>
              )}
              {r.error && <div className="error-box">{r.error}</div>}
            </div>
          ))}
        </div>
      )}

      <div className={`drop-zone ${dragActive ? 'drag-active' : ''}`}>
        템플릿 pptx / 요약 txt 파일을 여기로 드래그하면 자동으로 선택됩니다.
      </div>
    </>
  )
}
