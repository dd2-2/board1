import { useEffect, useRef, useState } from 'react'
import { getCurrentWebview } from '@tauri-apps/api/webview'

export interface DropTarget {
  exts: string[]
  onDrop: (paths: string[]) => void
}

/** 파일을 해당 버튼(data-drop="키") 위에 놓았을 때만 받고, 그 버튼이 허용하는 확장자만 넘긴다.
 *  반환값은 지금 끌고 있는 위치의 버튼 키(강조 표시용). */
export function useDropTargets(targets: Record<string, DropTarget>): string | null {
  const ref = useRef(targets)
  ref.current = targets
  const [hover, setHover] = useState<string | null>(null)

  useEffect(() => {
    // Tauri가 주는 좌표는 물리 픽셀 → CSS 픽셀로 바꿔서 그 위치의 버튼을 찾음
    const keyAt = (p: { x: number; y: number }) => {
      const dpr = window.devicePixelRatio || 1
      const el = document.elementFromPoint(p.x / dpr, p.y / dpr)?.closest<HTMLElement>('[data-drop]')
      const key = el?.dataset.drop
      return key && key in ref.current ? key : null
    }
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload
      if (p.type === 'enter' || p.type === 'over') {
        setHover(keyAt(p.position))
        return
      }
      setHover(null)
      if (p.type !== 'drop') return
      const key = keyAt(p.position)
      if (!key) return
      const target = ref.current[key]
      const files = p.paths.filter((f) => target.exts.some((x) => f.toLowerCase().endsWith(`.${x}`)))
      if (files.length > 0) target.onDrop(files)
    })
    return () => {
      unlisten.then((f) => f())
    }
  }, [])

  return hover
}
