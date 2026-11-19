import { describe, expect, it } from 'vitest'
import {
  addTab,
  closeTab,
  cycleTab,
  initialTabs,
  linkSaved,
  MAX_TABS,
  restoreTabs,
  updateSql,
} from './tabs'

describe('editor tabs', () => {
  it('names new tabs with the lowest free number', () => {
    let state = addTab(addTab(initialTabs()))
    expect(state.tabs.map((t) => t.title)).toEqual(['Query 1', 'Query 2', 'Query 3'])

    state = closeTab(state, state.tabs[1]!.id)
    state = addTab(state)
    expect(state.tabs.map((t) => t.title)).toEqual(['Query 1', 'Query 3', 'Query 2'])
    expect(state.activeId).toBe(state.tabs[2]!.id)
  })

  it('activates a neighbour on close and never leaves no tab', () => {
    let state = addTab(addTab(initialTabs()))
    const [first, second, third] = state.tabs
    state = { ...state, activeId: second!.id }

    state = closeTab(state, second!.id)
    expect(state.activeId).toBe(third!.id)
    state = closeTab(closeTab(state, first!.id), third!.id)
    expect(state.tabs).toHaveLength(1)
    expect(state.tabs[0]!.sql).toBe('')
  })

  it('caps the number of tabs and cycles through them', () => {
    let state = initialTabs()
    for (let i = 0; i < MAX_TABS + 5; i += 1) state = addTab(state)
    expect(state.tabs).toHaveLength(MAX_TABS)

    state = { ...state, activeId: state.tabs.at(-1)!.id }
    expect(cycleTab(state, 1).activeId).toBe(state.tabs[0]!.id)
    expect(cycleTab(state, -1).activeId).toBe(state.tabs.at(-2)!.id)
  })

  it('updates one draft and restores only well-formed state', () => {
    const start = addTab(initialTabs())
    const state = updateSql(start, start.tabs[1]!.id, 'SELECT 2')
    expect(state.tabs.map((t) => t.sql)).toEqual(['', 'SELECT 2'])

    const tab = { id: 'a', title: 'Mine', sql: 'SELECT 1' }
    expect(restoreTabs({ tabs: [tab, { id: 2 }], activeId: 'missing' })).toEqual({
      tabs: [tab],
      activeId: 'a',
    })
    expect(restoreTabs('garbage').tabs).toHaveLength(1)
  })

  it('opens and links saved queries under their name', () => {
    let state = addTab(initialTabs(), 'SELECT 1', { id: 's1', name: 'Top customers' })
    expect(state.tabs[1]).toMatchObject({ title: 'Top customers', savedQueryId: 's1' })

    state = linkSaved(state, state.tabs[0]!.id, { id: 's2', name: 'Orders today' })
    expect(state.tabs[0]).toMatchObject({ title: 'Orders today', savedQueryId: 's2' })
    expect(restoreTabs(state).tabs.map((t) => t.savedQueryId)).toEqual(['s2', 's1'])
    expect(
      restoreTabs({ tabs: [{ id: 'a', title: 'x', sql: '', savedQueryId: 7 }], activeId: 'a' })
        .tabs[0],
    ).toEqual({ id: 'a', title: 'x', sql: '' })
  })
})
