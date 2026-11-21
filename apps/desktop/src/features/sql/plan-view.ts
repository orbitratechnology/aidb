import type { PlanNode, QueryPlan } from '@aidb/contracts'

const number = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 })

/** "1,203", for rows and costs. */
export function amount(value: number): string {
  return number.format(Math.round(value))
}

/** "9.50 ms", "1.2 s". */
export function duration(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)} s`
  if (ms >= 100) return `${Math.round(ms)} ms`
  return `${ms.toFixed(2)} ms`
}

/** How far the estimate was off, as "12× more" or "8× fewer", when it was at least 10×. */
export function misestimate(node: PlanNode): string | null {
  if (node.actualRows === null) return null
  const larger = Math.max(node.actualRows, node.planRows)
  const smaller = Math.max(Math.min(node.actualRows, node.planRows), 1)
  if (larger < 100 || larger / smaller < 10) return null
  return `${amount(larger / smaller)}× ${node.actualRows > node.planRows ? 'more' : 'fewer'}`
}

/** The node that spent the most time on its own, when the plan was measured. */
export function slowestNode(plan: QueryPlan): number | null {
  let slowest: PlanNode | null = null
  for (const node of plan.nodes) {
    if (node.selfMs !== null && (slowest === null || node.selfMs > (slowest.selfMs ?? 0))) {
      slowest = node
    }
  }
  return slowest && (slowest.selfMs ?? 0) > 0 ? slowest.id : null
}

/** A node's share of the execution time, as a whole percentage. */
export function timeShare(node: PlanNode, plan: QueryPlan): number | null {
  if (node.selfMs === null || !plan.executionMs) return null
  return Math.round((node.selfMs / plan.executionMs) * 100)
}

/** The plan as indented text, for copying. */
export function planText(plan: QueryPlan): string {
  const lines = plan.nodes.flatMap((node) => {
    const indent = '  '.repeat(node.depth)
    let head = `${indent}-> ${node.label} (cost ${node.totalCost.toFixed(2)}, rows ${amount(node.planRows)}`
    if (node.actualRows !== null) head += `; actual rows ${amount(node.actualRows)}`
    if (node.actualMs !== null) head += `, ${duration(node.actualMs)}`
    head += ')'
    return [head, ...node.details.map((detail) => `${indent}     ${detail}`)]
  })
  if (plan.executionMs !== null) lines.push(`Execution: ${duration(plan.executionMs)}`)
  return lines.join('\n')
}

/** What to ask the assistant about a query's plan. */
export function assistantQuestion(sql: string): string {
  return `Why does this query take the time it does, and what would make it faster? Explain its plan and aidb's findings, with their evidence.\n\n\`\`\`sql\n${sql.trim()}\n\`\`\``
}
