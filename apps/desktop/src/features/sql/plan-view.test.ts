import type { PlanNode, QueryPlan } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import {
  assistantQuestion,
  duration,
  misestimate,
  planText,
  slowestNode,
  timeShare,
} from './plan-view'

function node(overrides: Partial<PlanNode>): PlanNode {
  return {
    id: 0,
    parent: null,
    depth: 0,
    nodeType: 'Seq Scan',
    label: 'Seq Scan on public.orders o',
    details: [],
    totalCost: 1943,
    planRows: 10,
    actualRows: null,
    loops: null,
    actualMs: null,
    selfMs: null,
    ...overrides,
  }
}

const measured: QueryPlan = {
  analyzed: true,
  planningMs: 0.1,
  executionMs: 20,
  truncated: false,
  suggestions: [],
  nodes: [
    node({
      id: 0,
      nodeType: 'Hash Join',
      label: 'Hash Join',
      actualRows: 5,
      actualMs: 20,
      selfMs: 4,
    }),
    node({
      id: 1,
      parent: 0,
      depth: 1,
      actualRows: 52000,
      planRows: 5,
      actualMs: 16,
      selfMs: 16,
      details: ['Filter: (o.total > 1)'],
    }),
  ],
}

describe('plan display', () => {
  it('finds the slowest node and its share of the time', () => {
    expect(slowestNode(measured)).toBe(1)
    expect(timeShare(measured.nodes[1]!, measured)).toBe(80)
    expect(slowestNode({ ...measured, nodes: [node({})] })).toBeNull()
  })

  it('says when an estimate was far off', () => {
    expect(misestimate(measured.nodes[1]!)).toBe('10,400× more')
    expect(misestimate(node({ actualRows: 12, planRows: 10 }))).toBeNull()
    expect(misestimate(node({ planRows: 10 }))).toBeNull()
  })

  it('copies as indented text', () => {
    expect(planText(measured)).toBe(
      [
        '-> Hash Join (cost 1943.00, rows 10; actual rows 5, 20.00 ms)',
        '  -> Seq Scan on public.orders o (cost 1943.00, rows 5; actual rows 52,000, 16.00 ms)',
        '       Filter: (o.total > 1)',
        'Execution: 20.00 ms',
      ].join('\n'),
    )
    expect(duration(1500)).toBe('1.5 s')
  })

  it('asks the assistant about the query itself', () => {
    expect(assistantQuestion(' SELECT 1 ')).toContain('```sql\nSELECT 1\n```')
  })
})
