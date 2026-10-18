import type { ReactNode } from 'react'

/** 가로로 나란히 놓이는 단계 카드: 번호 + 제목 + 짧은 설명 + 그 단계의 버튼들 */
export default function StepCard({
  num,
  title,
  desc,
  active,
  children,
}: {
  num: number
  title: string
  desc?: string
  active?: boolean
  children?: ReactNode
}) {
  return (
    <div className={`step ${active ? 'drag-active' : ''}`}>
      <div className="step-head">
        <span className="step-num">{num}</span>
        <span className="step-title">{title}</span>
      </div>
      {desc && <div className="step-desc">{desc}</div>}
      {children && <div className="step-body">{children}</div>}
    </div>
  )
}
