import { describe, expect, it } from 'vitest'
import { QueryPlanSchema } from './advisor'

describe('advisor contracts', () => {
  it('reads a measured plan with a suggestion', () => {
    const plan = QueryPlanSchema.parse({
      analyzed: true,
      planningMs: 0.1,
      executionMs: 9.7,
      truncated: false,
      nodes: [
        {
          id: 0,
          parent: null,
          depth: 0,
          nodeType: 'Seq Scan',
          label: 'Seq Scan on public.orders o',
          details: ['Filter: (o.customer_id = 42)'],
          totalCost: 1943,
          planRows: 12,
          actualRows: 12,
          loops: 1,
          actualMs: 9.5,
          selfMs: 9.5,
        },
      ],
      suggestions: [
        {
          kind: 'missing_index',
          severity: 'warning',
          title: 'An index on public.orders (customer_id) may help',
          detail: '…',
          evidence: ['Seq Scan on public.orders o read 100,000 rows'],
          node: 0,
          sql: 'CREATE INDEX CONCURRENTLY ON public.orders (customer_id);',
        },
      ],
    })
    expect(plan.suggestions[0]?.node).toBe(0)
    expect(
      QueryPlanSchema.safeParse({ ...plan, suggestions: [{ ...plan.suggestions[0], kind: 'x' }] })
        .success,
    ).toBe(false)
  })
})
