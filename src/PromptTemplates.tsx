import { useState } from 'react'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import withOriginal from './assets/prompts/with-original.txt?raw'
import withoutOriginal from './assets/prompts/without-original.txt?raw'

const PROMPTS = [
  { key: 'with', label: '원문 표시 포함 (--아래 교수님원문--)', text: withOriginal },
  { key: 'without', label: '원문 표시 없음', text: withoutOriginal },
]

export default function PromptTemplates() {
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  async function copy(key: string, text: string) {
    await writeText(text)
    setCopiedKey(key)
    setTimeout(() => setCopiedKey((k) => (k === key ? null : k)), 1500)
  }

  return (
    <div className="prompt-section">
      <h2>GPT 프롬프트 (PDF+녹취 → 페이지별 대본 생성용)</h2>
      {PROMPTS.map((p) => (
        <div className="prompt-card" key={p.key}>
          <span>{p.label}</span>
          <button onClick={() => copy(p.key, p.text)}>
            {copiedKey === p.key ? '복사됨' : '복사'}
          </button>
        </div>
      ))}
    </div>
  )
}
