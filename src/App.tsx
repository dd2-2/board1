import { useEffect, useState, type ReactNode } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import PromptTemplates from './PromptTemplates'
import SummaryTab from './SummaryTab'
import StepCard from './StepCard'
import { useDropTargets } from './useDropTargets'

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
  scriptPath?: string
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

function NotesTab({ ai, apiCard }: { ai: boolean; apiCard: ReactNode }) {
  const [batch, setBatch] = useState<Batch>({ pairs: [], unmatchedTxt: [] })
  const { pairs, unmatchedTxt } = batch
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [batchResults, setBatchResults] = useState<BatchResult[] | null>(null)

  function addFiles(newPptx: string[], newTxt: string[]) {
    setBatch((prev) => addFilesToBatch(prev.pairs, prev.unmatchedTxt, newPptx, newTxt))
    setBatchResults(null)
    setError(null)
  }

  // 각 버튼 위에 놓은 파일만, 그 버튼이 받는 형식만
  const dropHover = useDropTargets({
    pptx: { exts: ['pptx'], onDrop: (p) => addFiles(p, []) },
    txt: { exts: ['txt', 'hwp', 'hwpx'], onDrop: (p) => addFiles([], p) },
  })
  const dropCls = (key: string) => `file-btn ${dropHover === key ? 'drop-hover' : ''}`

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
      filters: [{ name: 'TXT / HWP / HWPX', extensions: ['txt', 'hwp', 'hwpx'] }],
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
    for (const [i, pair] of runnablePairs.entries()) {
      const outputPath = outputPathFor(pair.pptxPath)
      let scriptPath: string | undefined
      try {
        // GPT 자동 모드: 원본 녹취 TXT → GPT가 "Np" 대본 TXT를 만들고, 그 대본으로 메모 입력
        if (ai) {
          setProgress(`GPT 대본 생성 중... (${i + 1}/${runnablePairs.length})`)
          scriptPath = await invoke<string>('ai_generate_script', {
            pptxPath: pair.pptxPath,
            transcriptPath: pair.txtPath,
          })
        }
        const summary = await invoke<ImportSummary>('import_notes', {
          pptxPath: pair.pptxPath,
          outputPath,
          txtPath: scriptPath ?? pair.txtPath,
          keepOriginal,
        })
        results.push({ pptxPath: pair.pptxPath, outputPath, scriptPath, summary })
      } catch (e) {
        results.push({ pptxPath: pair.pptxPath, outputPath, scriptPath, error: String(e) })
      }
      setBatchResults([...results])
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
              <PromptTemplates />
            </StepCard>
            <StepCard num={2} title="ChatGPT로 대본" desc="PPTX·녹취 + 프롬프트" />
          </>
        )}
        <StepCard num={ai ? 2 : 3} title="파일 선택" desc={ai ? 'PPTX + 원본 녹취' : 'PPTX + 대본 TXT'}>
          <button className={dropCls('pptx')} data-drop="pptx" onClick={pickPptx}>
            PPTX 추가
          </button>
          <button className={dropCls('txt')} data-drop="txt" onClick={pickTxt}>
            {ai ? '녹취 추가' : 'TXT 추가'}
          </button>
        </StepCard>
        <StepCard num={ai ? 3 : 4} title="실행" desc="→ 녹취추가_원본.pptx">
          <button className="primary" onClick={() => run(false)} disabled={runnablePairs.length === 0 || running}>
            {running
              ? progress || '실행 중...'
              : `${ai ? 'GPT로 ' : ''}메모 입력${runnablePairs.length > 1 ? ` (${runnablePairs.length}건)` : ''}`}
          </button>
          <button
            className="secondary"
            onClick={() => run(true)}
            disabled={runnablePairs.length === 0 || running}
            title="기존 메모(교수님원문)를 지우지 않고 새 내용 아래로 보존"
          >
            교수님원문 보존
          </button>
        </StepCard>
      </div>

      {pairs.length > 0 && (
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
                <option value="">(TXT 없음)</option>
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
      )}

      {unmatchedTxt.length > 0 && (
        <div className="pair-list">
          {unmatchedTxt.map((t) => (
            <div className="pair-row" key={t}>
              <div className="pair-pptx empty" title={t}>
                짝 없는 TXT: {fileName(t)}
              </div>
              <button className="pair-remove" onClick={() => removeUnmatchedTxt(t)} title="목록에서 제거">
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

      {error && <div className="error-box">{error}</div>}

      {batchResults && (
        <div className="result">
          {batchResults.map((r) => (
            <div className="batch-result" key={r.pptxPath}>
              <h3 className="batch-result-title">{fileName(r.pptxPath)}</h3>
              {r.scriptPath && <div className="batch-result-note">GPT 대본: {fileName(r.scriptPath)}</div>}
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
    </>
  )
}

type Tab = 'notes' | 'summary'

interface AiConfig {
  enabled: boolean
  apiKey: string
}

const MODES: { tab: Tab; title: string; desc: string }[] = [
  { tab: 'notes', title: '스토리보드용(설계자)', desc: 'PPT 슬라이드 메모에 녹취 입력' },
  { tab: 'summary', title: '다운로드교안용', desc: '템플릿으로 요약 교안 PPT 생성' },
]

export default function App() {
  // 처음 실행하면 모드 선택 화면(null)부터
  const [tab, setTab] = useState<Tab | null>(null)
  const [ai, setAi] = useState<AiConfig>({ enabled: false, apiKey: '' })

  useEffect(() => {
    invoke<AiConfig>('get_ai_config').then(setAi)
  }, [])

  /** 변경 즉시 exe 옆 board1_config.json에 저장 */
  function updateAi(patch: Partial<AiConfig>) {
    const next = { ...ai, ...patch }
    setAi(next)
    invoke('set_ai_config', { config: next })
  }

  // GPT 자동 모드의 1단계 카드 (두 탭 공용, 모델은 gpt-5.6-luna 고정)
  const apiCard = (
    <StepCard num={1} title="API 키" desc="처음 한 번">
      <input
        type="password"
        className="text-input"
        placeholder="sk-..."
        value={ai.apiKey}
        onChange={(e) => setAi({ ...ai, apiKey: e.target.value })}
        onBlur={() => updateAi({})}
      />
    </StepCard>
  )

  return (
    <main className="app">
      <div className="header-row">
        <div className="tabs">
          {tab &&
            MODES.map((m) => (
              <button key={m.tab} className={tab === m.tab ? 'active' : ''} onClick={() => setTab(m.tab)}>
                {m.title}
              </button>
            ))}
        </div>
        <label className="switch" title="OpenAI API(gpt-5.6-luna)로 ChatGPT 사이트 없이 앱 안에서 처리">
          <input type="checkbox" checked={ai.enabled} onChange={(e) => updateAi({ enabled: e.target.checked })} />
          <span className="switch-track" />
          GPT 자동
        </label>
      </div>
      {tab === null ? (
        <div className="steps mode-select">
          {MODES.map((m) => (
            <button key={m.tab} className="mode-card" onClick={() => setTab(m.tab)}>
              <span className="mode-title">{m.title}</span>
              <span className="mode-desc">{m.desc}</span>
            </button>
          ))}
        </div>
      ) : tab === 'notes' ? (
        <NotesTab ai={ai.enabled} apiCard={apiCard} />
      ) : (
        <SummaryTab ai={ai.enabled} apiCard={apiCard} />
      )}
    </main>
  )
}
