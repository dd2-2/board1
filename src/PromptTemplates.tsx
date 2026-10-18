import { useState } from 'react'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import withOriginal from './assets/prompts/with-original.txt?raw'
import withoutOriginal from './assets/prompts/without-original.txt?raw'

const PROMPTS = [
  { key: 'with', label: '원문 표시 포함', text: withOriginal },
  { key: 'without', label: '원문 표시 없음', text: withoutOriginal },
]

/** 1단계 카드 안에 들어가는 프롬프트 복사 버튼들 */
export default function PromptTemplates() {
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  async function copy(key: string, text: string) {
    await writeText(text)
    setCopiedKey(key)
    setTimeout(() => setCopiedKey((k) => (k === key ? null : k)), 1500)
  }

  return (
    <>
      {PROMPTS.map((p) => (
        <button key={p.key} onClick={() => copy(p.key, p.text)}>
          {copiedKey === p.key ? '복사됨' : p.label}
        </button>
      ))}
    </>
  )
}
