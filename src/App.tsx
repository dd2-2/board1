import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import PromptTemplates from './PromptTemplates'

interface PageResult {
  page: number
  message: string
}

interface ImportSummary {
  applied: PageResult[]
  failed: PageResult[]
}

function fileName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

export default function App() {
  const [pptxPath, setPptxPath] = useState<string | null>(null)
  const [txtPath, setTxtPath] = useState<string | null>(null)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<ImportSummary | null>(null)
  const [dragActive, setDragActive] = useState(false)

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over') {
        setDragActive(true)
        return
      }
      if (event.payload.type !== 'drop') {
        setDragActive(false)
        return
      }
      setDragActive(false)
      for (const p of event.payload.paths) {
        const lower = p.toLowerCase()
        if (lower.endsWith('.pptx')) setPptxPath(p)
        else if (lower.endsWith('.txt')) setTxtPath(p)
      }
      setResult(null)
      setError(null)
    })
    return () => {
      unlisten.then((f) => f())
    }
  }, [])

  async function pickPptx() {
    const selected = await open({
      multiple: false,
      filters: [{ name: 'PowerPoint', extensions: ['pptx'] }],
    })
    if (typeof selected === 'string') {
      setPptxPath(selected)
      setResult(null)
      setError(null)
    }
  }

  async function pickTxt() {
    const selected = await open({
      multiple: false,
      filters: [{ name: 'Text', extensions: ['txt'] }],
    })
    if (typeof selected === 'string') {
      setTxtPath(selected)
      setResult(null)
      setError(null)
    }
  }

  async function run() {
    if (!pptxPath || !txtPath) return
    setRunning(true)
    setError(null)
    setResult(null)
    try {
      const summary = await invoke<ImportSummary>('import_notes', {
        pptxPath,
        txtPath,
      })
      setResult(summary)
    } catch (e) {
      setError(String(e))
    } finally {
      setRunning(false)
    }
  }

  return (
    <div className="app">
      <div>
        <h1>board1 — PPT 메모 자동 입력</h1>
      </div>

      <PromptTemplates />

      <div className="field">
        <label>PPTX 파일</label>
        <div className="picker-row">
          <div className={`picker-path ${pptxPath ? '' : 'empty'}`} title={pptxPath ?? ''}>
            {pptxPath ? fileName(pptxPath) : '선택된 파일 없음'}
          </div>
          <button onClick={pickPptx}>찾아보기</button>
        </div>
      </div>

      <div className="field">
        <label>녹취 TXT 파일</label>
        <div className="picker-row">
          <div className={`picker-path ${txtPath ? '' : 'empty'}`} title={txtPath ?? ''}>
            {txtPath ? fileName(txtPath) : '선택된 파일 없음'}
          </div>
          <button onClick={pickTxt}>찾아보기</button>
        </div>
      </div>

      <div className="run-row">
        <button className="primary" onClick={run} disabled={!pptxPath || !txtPath || running}>
          {running ? '실행 중...' : '메모 입력 실행'}
        </button>
      </div>

      {error && <div className="error-box">{error}</div>}

      {result && (
        <div className="result">
          <div className="result-section">
            <h2>적용됨 ({result.applied.length})</h2>
            <div className="result-list">
              {result.applied.map((r) => (
                <div className="result-item ok" key={`ok-${r.page}`}>
                  <span className="badge">완료</span>
                  <span>{r.page}p</span>
                  <span className="message">{r.message}</span>
                </div>
              ))}
            </div>
          </div>
          {result.failed.length > 0 && (
            <div className="result-section">
              <h2>건너뜀 ({result.failed.length})</h2>
              <div className="result-list">
                {result.failed.map((r) => (
                  <div className="result-item fail" key={`fail-${r.page}`}>
                    <span className="badge">실패</span>
                    <span>{r.page}p</span>
                    <span className="message">{r.message}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      <div className={`drop-zone ${dragActive ? 'drag-active' : ''}`}>
        pptx / txt 파일을 여기로 드래그하면 자동으로 선택됩니다.
      </div>
    </div>
  )
}
