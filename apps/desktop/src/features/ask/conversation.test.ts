import { ASSISTANT_LIMITS, type TaskOutcome } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import { emptyActivity } from './activity'
import { attachmentProblem, formatSize, historyFor, type Turn } from './conversation'

function turn(id: number, answer: string | null, state: Turn['state'] = 'done'): Turn {
  return {
    id: String(id),
    question: `q${id}`,
    attachments: [],
    selected: [],
    activity: emptyActivity(),
    state,
    outcome:
      answer === null
        ? undefined
        : ({ taskId: 't', status: 'succeeded', answer, notice: null } as unknown as TaskOutcome),
  }
}

describe('conversation', () => {
  it('sends the most recent answered turns as history', () => {
    const turns = [
      ...Array.from({ length: 8 }, (_, i) => turn(i, `a${i}`)),
      turn(8, ''),
      turn(9, null, 'error'),
      turn(10, null, 'running'),
    ]
    const history = historyFor(turns)
    expect(history).toHaveLength(ASSISTANT_LIMITS.maxHistory)
    expect(history[0]).toEqual({ question: 'q2', answer: 'a2' })
    expect(history.at(-1)).toEqual({ question: 'q7', answer: 'a7' })
  })

  it('checks files before reading them', () => {
    expect(attachmentProblem({ name: 'orders.CSV', size: 10 }, [])).toBeNull()
    expect(attachmentProblem({ name: 'photo.png', size: 10 }, [])).toMatch(/cannot be attached/)
    expect(
      attachmentProblem({ name: 'big.sql', size: ASSISTANT_LIMITS.attachmentMaxBytes + 1 }, []),
    ).toMatch(/too large/)
    expect(attachmentProblem({ name: 'a.md', size: 1 }, [{ name: 'a.md' }])).toMatch(/already/)
    const full = Array.from({ length: ASSISTANT_LIMITS.maxAttachments }, (_, i) => ({
      name: `${i}.txt`,
    }))
    expect(attachmentProblem({ name: 'x.txt', size: 1 }, full)).toMatch(/at most/)
  })

  it('formats sizes', () => {
    expect(formatSize(512)).toBe('512 B')
    expect(formatSize(2048)).toBe('2 KB')
    expect(formatSize(1536 * 1024)).toBe('1.5 MB')
  })
})
