import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import PromptTemplates from './PromptTemplates'
import SummaryTab from './SummaryTab'

interface PageResult {
  page: number
  message: string
}

interface ImportSummary {
  applied: PageResult[]
  failed: PageResult[]
}

interface Pair {
  pptxPath: string
  txtPath: string | null
}

interface BatchResult {
  pptxPath: string
  outputPath: string
  summary?: ImportSummary
  error?: string
}

function fileName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/** 원본을 건드리지 않도록 같은 폴더에 "녹취추가_" 접두사를 붙인 경로를 만든다. */
function outputPathFor(pptxPath: string): string {
  const idx = Math.max(pptxPath.lastIndexOf('\\'), pptxPath.lastIndexOf('/'))
  const dir = idx === -1 ? '' : pptxPath.slice(0, idx + 1)
  const name = idx === -1 ? pptxPath : pptxPath.slice(idx + 1)
  return `${dir}녹취추가_${name}`
}

/** 파일명에서 맨 처음 나오는 숫자를 페이지/주차 매칭 키로 사용 (예: "3주차 ...pptx" / "03주차_슬라이드별.txt" 둘 다 3) */
function leadingNumber(path: string): number | null {
  const m = fileName(path).match(/\d+/)
  if (!m) return null
  return parseInt(m[0], 10)
}

function addFilesToBatch(
  pairs: Pair[],
  unmatchedTxt: string[],
  newPptx: string[],
  newTxt: string[],
): { pairs: Pair[]; unmatchedTxt: string[] } {
  const nextPairs = [...pairs]
  let nextUnmatched = [...unmatchedTxt]

  const existingPptx = new Set(nextPairs.map((p) => p.pptxPath))
  for (const p of newPptx) {
    if (existingPptx.has(p)) continue
    existingPptx.add(p)
    let matched: string | null = null
    const n = leadingNumber(p)
    if (n !== null) {
      const idx = nextUnmatched.findIndex((t) => leadingNumber(t) === n)
      if (idx !== -1) {
        matched = nextUnmatched[idx]
        nextUnmatched.splice(idx, 1)
      }
    }
    nextPairs.push({ pptxPath: p, txtPath: matched })
  }

  const usedTxt = new Set(nextPairs.map((p) => p.txtPath).filter((t): t is string => !!t))
  for (const t of newTxt) {
    if (usedTxt.has(t) || nextUnmatched.includes(t)) continue
    const n = leadingNumber(t)
    const idx = n === null ? -1 : nextPairs.findIndex((p) => p.txtPath === null && leadingNumber(p.pptxPath) === n)
    if (idx !== -1) {
      nextPairs[idx] = { ...nextPairs[idx], txtPath: t }
      usedTxt.add(t)
    } else {
      nextUnmatched.push(t)
    }
  }

  return { pairs: nextPairs, unmatchedTxt: nextUnmatched }
}

interface Batch {
  pairs: Pair[]
  unmatchedTxt: string[]
}

function NotesTab() {
  const [batch, setBatch] = useState<Batch>({ pairs: [], unmatchedTxt: [] })
  const { pairs, unmatchedTxt } = batch
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [batchResults, setBatchResults] = useState<BatchResult[] | null>(null)
  const [dragActive, setDragActive] = useState(false)

  function addFiles(newPptx: string[], newTxt: string[]) {
    setBatch((prev) => addFilesToBatch(prev.pairs, prev.unmatchedTxt, newPptx, newTxt))
    setBatchResults(null)
    setError(null)
  }

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
      const newPptx: string[] = []
      const newTxt: string[] = []
      for (const p of event.payload.paths) {
        const lower = p.toLowerCase()
        if (lower.endsWith('.pptx')) newPptx.push(p)
        else if (lower.endsWith('.txt')) newTxt.push(p)
      }
      addFiles(newPptx, newTxt)
    })
    return () => {
      unlisten.then((f) => f())
    }
  }, [])

  async function pickPptx() {
    const selected = await open({
      multiple: true,
      filters: [{ name: 'PowerPoint', extensions: ['pptx'] }],
    })
    if (Array.isArray(selected) && selected.length > 0) {
      addFiles(selected, [])
    } else if (typeof selected === 'string') {
      addFiles([selected], [])
    }
  }

  async function pickTxt() {
    const selected = await open({
      multiple: true,
      filters: [{ name: 'Text', extensions: ['txt'] }],
    })
    if (Array.isArray(selected) && selected.length > 0) {
      addFiles([], selected)
    } else if (typeof selected === 'string') {
      addFiles([], [selected])
    }
  }

  function removePair(pptxPath: string) {
    setBatch((prev) => {
      const target = prev.pairs.find((p) => p.pptxPath === pptxPath)
      return {
        pairs: prev.pairs.filter((p) => p.pptxPath !== pptxPath),
        unmatchedTxt: target?.txtPath ? [...prev.unmatchedTxt, target.txtPath] : prev.unmatchedTxt,
      }
    })
  }

  function removeUnmatchedTxt(txtPath: string) {
    setBatch((prev) => ({ ...prev, unmatchedTxt: prev.unmatchedTxt.filter((t) => t !== txtPath) }))
  }

  function reassignTxt(pptxPath: string, newTxtPath: string) {
    setBatch((prev) => {
      const target = prev.pairs.find((p) => p.pptxPath === pptxPath)
      let nextUnmatched = prev.unmatchedTxt.filter((t) => t !== newTxtPath)
      if (target?.txtPath) {
        nextUnmatched = [...nextUnmatched, target.txtPath]
      }
      return {
        pairs: prev.pairs.map((p) => (p.pptxPath === pptxPath ? { ...p, txtPath: newTxtPath || null } : p)),
        unmatchedTxt: nextUnmatched,
      }
    })
  }

  const runnablePairs = pairs.filter((p): p is Pair & { txtPath: string } => !!p.txtPath)

  async function run(keepOriginal: boolean) {
    if (runnablePairs.length === 0) return
    setRunning(true)
    setError(null)
    setBatchResults(null)
    const results: BatchResult[] = []
    for (const pair of runnablePairs) {
      const outputPath = outputPathFor(pair.pptxPath)
      try {
        const summary = await invoke<ImportSummary>('import_notes', {
          pptxPath: pair.pptxPath,
          outputPath,
          txtPath: pair.txtPath,
          keepOriginal,
        })
        results.push({ pptxPath: pair.pptxPath, outputPath, summary })
      } catch (e) {
        results.push({ pptxPath: pair.pptxPath, outputPath, error: String(e) })
      }
      setBatchResults([...results])
    }
    setRunning(false)
  }

  return (
    <>
      <PromptTemplates />

      <div className="field">
        <label>PPTX / 녹취 TXT 파일 (여러 건 선택 가능, 파일명 숫자로 자동 매칭)</label>
        <div className="picker-row">
          <button onClick={pickPptx}>PPTX 추가</button>
          <button onClick={pickTxt}>TXT 추가</button>
        </div>
      </div>

      {pairs.length > 0 && (
        <div className="field">
          <div className="pair-list">
            {pairs.map((pair) => (
              <div className="pair-row" key={pair.pptxPath}>
                <div className="pair-pptx" title={pair.pptxPath}>
                  {fileName(pair.pptxPath)}
                </div>
                <span className="pair-arrow">→</span>
                <select
                  className={`pair-select ${pair.txtPath ? '' : 'empty'}`}
                  value={pair.txtPath ?? ''}
                  onChange={(e) => reassignTxt(pair.pptxPath, e.target.value)}
                >
                  <option value="">(매칭된 TXT 없음)</option>
                  {pair.txtPath && <option value={pair.txtPath}>{fileName(pair.txtPath)}</option>}
                  {unmatchedTxt.map((t) => (
                    <option key={t} value={t}>
                      {fileName(t)}
                    </option>
                  ))}
                </select>
                <button className="pair-remove" onClick={() => removePair(pair.pptxPath)} title="목록에서 제거">
                  ✕
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {unmatchedTxt.length > 0 && (
        <div className="field">
          <label>매칭 안 된 TXT ({unmatchedTxt.length}) — 위 목록에서 PPTX에 직접 지정하세요</label>
          <div className="pair-list">
            {unmatchedTxt.map((t) => (
              <div className="pair-row" key={t}>
                <div className="pair-pptx empty" title={t}>
                  {fileName(t)}
                </div>
                <button className="pair-remove" onClick={() => removeUnmatchedTxt(t)} title="목록에서 제거">
                  ✕
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="run-row">
        <button
          className="primary"
          onClick={() => run(false)}
          disabled={runnablePairs.length === 0 || running}
        >
          {running ? '실행 중...' : `메모 입력 실행${runnablePairs.length > 1 ? ` (${runnablePairs.length}건)` : ''}`}
        </button>
        <button
          onClick={() => run(true)}
          disabled={runnablePairs.length === 0 || running}
          title="기존 메모에 이미 있는 교수님원문은 지우지 않고 새 내용 아래로 밀어서 보존합니다."
        >
          교수님원문
        </button>
      </div>

      {error && <div className="error-box">{error}</div>}

      {batchResults && (
        <div className="result">
          {batchResults.map((r) => (
            <div className="batch-result" key={r.pptxPath}>
              <h3 className="batch-result-title">{fileName(r.pptxPath)}</h3>
              {r.summary && (
                <div className="batch-result-saved">→ 저장됨: {fileName(r.outputPath)}</div>
              )}
              {r.error && <div className="error-box">{r.error}</div>}
              {r.summary && (
                <>
                  <div className="result-section">
                    <h2>적용됨 ({r.summary.applied.length})</h2>
                    <div className="result-list">
                      {r.summary.applied.map((res) => (
                        <div className="result-item ok" key={`ok-${r.pptxPath}-${res.page}`}>
                          <span className="badge">완료</span>
                          <span>{res.page}p</span>
                          <span className="message">{res.message}</span>
                        </div>
                      ))}
                    </div>
                  </div>
                  {r.summary.failed.length > 0 && (
                    <div className="result-section">
                      <h2>건너뜀 ({r.summary.failed.length})</h2>
                      <div className="result-list">
                        {r.summary.failed.map((res) => (
                          <div className="result-item fail" key={`fail-${r.pptxPath}-${res.page}`}>
                            <span className="badge">실패</span>
                            <span>{res.page}p</span>
                            <span className="message">{res.message}</span>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </>
              )}
            </div>
          ))}
        </div>
      )}

      <div className={`drop-zone ${dragActive ? 'drag-active' : ''}`}>
        pptx / txt 파일을 여기로 드래그하면 자동으로 선택됩니다. (여러 건 동시에 가능)
      </div>
    </>
  )
}

type Tab = 'notes' | 'summary'

export default function App() {
  const [tab, setTab] = useState<Tab>('notes')
  return (
    <div className="layout">
      <aside className="guide">
        <h2>사용법</h2>
        {tab === 'notes' ? (
          <ol>
            <li>오른쪽 <span className="hi">GPT 프롬프트</span> 복사(복사 버튼).</li>
            <li>강의안(PPTX), 녹취록(TXT)을 ChatGPT 채팅창에 복사된 프롬프트(1)와 함께 일괄 업로드.</li>
            <li>결과물(TXT) 파일 다운로드.</li>
            <li>오른쪽에서 원본 강의안(PPTX)과 방금 받은 정리된 TXT 파일 선택 (여러 건도 한번에 가능, 파일명 숫자로 자동 매칭).</li>
            <li>하단 <span className="hi">"메모 입력 실행"</span> 클릭 (기존 메모를 보존하려면 <span className="hi">"교수님원문"</span>).</li>
            <li>원본 PPTX와 같은 폴더에 <span className="hi">녹취추가_원본이름.pptx</span>로 저장됨.</li>
          </ol>
        ) : (
          <ol>
            <li>오른쪽 <span className="hi">GPT 프롬프트</span> 복사(복사 버튼).</li>
            <li>스토리보드, 녹취록을 ChatGPT에 프롬프트와 함께 업로드해 요약 TXT를 받음.</li>
            <li><span className="hi">디자인 템플릿 PPTX</span>(3페이지, {'{{차시타이틀}}/{{학습목표}}/{{대주제}}'} 등 자리표시 포함)와 요약 TXT 선택 (TXT 여러 건 가능).</li>
            <li><span className="hi">"PPT 만들기"</span> 클릭.</li>
            <li>TXT와 같은 폴더에 <span className="hi">요약완성_TXT이름.pptx</span>로 저장됨.</li>
          </ol>
        )}
      </aside>
      <main className="app">
        <h1>PPT + 녹취록메모</h1>
        <div className="tabs">
          <button className={tab === 'notes' ? 'active' : ''} onClick={() => setTab('notes')}>
            PPT 메모 입력
          </button>
          <button className={tab === 'summary' ? 'active' : ''} onClick={() => setTab('summary')}>
            요약 PPT 생성
          </button>
        </div>
        {tab === 'notes' ? <NotesTab /> : <SummaryTab />}
      </main>
    </div>
  )
}
