import type { ReactNode } from 'react'

/** 가로로 나란히 놓이는 단계 카드: 번호 + 제목(+오른쪽 위 보조 버튼) + 짧은 설명 + 그 단계의 버튼들 */
export default function StepCard({
  num,
  title,
  desc,
  action,
  children,
}: {
  num: number
  title: string
  desc?: ReactNode
  action?: ReactNode
  children?: ReactNode
}) {
  return (
    <div className="step">
      <div className="step-head">
        <span className="step-num">{num}</span>
        <span className="step-title">{title}</span>
        {action && <span className="step-action">{action}</span>}
      </div>
      {desc && <div className="step-desc">{desc}</div>}
      {children && <div className="step-body">{children}</div>}
    </div>
  )
}
