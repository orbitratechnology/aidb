import { describe, expect, it } from 'vitest'
import { needsAttention, stepsText } from './history'

describe('migration history', () => {
  it('counts the steps that ran', () => {
    expect(stepsText({ steps: 4, appliedSteps: 4 })).toBe('4 steps')
    expect(stepsText({ steps: 4, appliedSteps: 0 })).toBe('0 of 4 steps')
    expect(stepsText({ steps: 1, appliedSteps: 1 })).toBe('1 step')
  })

  it('flags schemas left between two states', () => {
    expect(needsAttention('partial')).toBe(true)
    expect(needsAttention('uncertain')).toBe(true)
    expect(needsAttention('failed')).toBe(false)
    expect(needsAttention('succeeded')).toBe(false)
  })
})
