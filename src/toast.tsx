import { createContext, useCallback, useContext, useState, type ReactNode } from 'react'

export type ToastKind = 'ok' | 'warn'

interface Toast {
  id: number
  text: string
  detail?: string
  kind: ToastKind
}

type Notify = (text: string, kind?: ToastKind, detail?: string) => void

const ToastContext = createContext<Notify>(() => {})

/** 완료 알림: 왼쪽 아래에 잠깐 떴다 사라짐 (클릭하면 바로 닫힘, 마우스를 올리면 자세한 내용 툴팁) */
export const useToast = () => useContext(ToastContext)

const DURATION_MS = 6000

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([])

  const close = useCallback((id: number) => setToasts((list) => list.filter((t) => t.id !== id)), [])

  const notify = useCallback<Notify>(
    (text, kind = 'ok', detail) => {
      const id = Date.now() + Math.random()
      setToasts((list) => [...list, { id, text, kind, detail }])
      setTimeout(() => close(id), DURATION_MS)
    },
    [close],
  )

  return (
    <ToastContext.Provider value={notify}>
      {children}
      <div className="toasts">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`} title={t.detail} onClick={() => close(t.id)}>
            {t.text}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  )
}
