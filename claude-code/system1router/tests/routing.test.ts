import { expect, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'
import type { On } from 'claude-code'

const PANEL = {
  status: { enabled: true, provider: 'jev', providerReady: true, rules: 12, claudeCodeRules: 6, consentMode: 'interactive', swap: 'model', efforts: { question_simple: 'low', code_hard: 'xhigh' }, configPath: '/c/config.json', providerLog: '/s/provider-debug.jsonl' },
  stats: { calls: 10, models: { 'anthropic/claude-sonnet-5-5': 7, 'anthropic/opus': 3 }, answered: 4, accepted: 3, advised: 1, byHost: { 'claude-code': 10 }, efforts: {} },
  preferences: [{ key: 'anthropic/claude-sonnet-5-5 → anthropic/haiku', accept: false, current: 'anthropic/claude-sonnet-5-5', suggested: 'anthropic/haiku', createdAt: 1 }],
}

const ASK = {
  action: 'ask', record: true, current: 'claude-sonnet-5-5', model: 'opus', modelIds: ['claude-opus-5-5'], variant: 'high', remembered: null,
  routingClass: 'code_hard', confidence: 0.8, reason: 'Complex or high-risk change', code: 'consent_requested',
}

/** An effort swap: same model, another effort. */
const EFFORT = {
  action: 'ask', record: true, kind: 'effort', current: 'claude-sonnet-5-5', currentEffort: null, model: 'claude-sonnet-5-5', modelIds: ['claude-sonnet-5-5'], variant: 'xhigh', effort: 'xhigh', remembered: null,
  routingClass: 'code_hard', confidence: 0.8, reason: 'code_hard work takes xhigh effort', code: 'consent_requested',
}

const UNCHANGED = { action: 'none', record: false, current: 'claude-sonnet-5-5', currentEffort: 'xhigh', routingClass: 'code_moderate', code: 'effort_unchanged' }

type Fake = {
  decision: object
  /** What `system1router panel` answers. */
  panel: object
  /** What `$.clock.now()` answers. */
  now: number
  /** The label picked in the dialog; undefined dismisses it. */
  answer: string | undefined
  calls: string[][]
  stdin: string[]
  /** The model and effort the last model request was sent with. */
  sent: { model: string; effort: unknown }
  /** Every model a request was sent to, and the ids the API refuses. */
  tried: string[]
  unknown: string[]
  toasts: string[]
  /** SYSTEM1ROUTER_CLAUDE_MOD as the mod set it. */
  env: string | undefined
  /** Ids of the open panes. */
  panes: string[]
  /** Prompt texts as the engine received them. */
  submitted: string[]
}

/** The engine beneath the mod: the binary, the session, the dialog and the turn. */
function engine(on: On, decision: object, answer?: string): Fake {
  const state: Fake = { decision, panel: PANEL, now: 1_000, answer, calls: [], stdin: [], sent: { model: '', effort: undefined }, tried: [], unknown: [], toasts: [], env: undefined, panes: [], submitted: [] }
  on('process.run', ($, e) => {
    state.calls.push([...e.argv.slice(1)])
    if (e.init?.stdin) state.stdin.push(e.init.stdin)
    const command = e.argv[1]
    const stdout = command === 'decide' ? JSON.stringify(state.decision) : command === 'panel' ? JSON.stringify(state.panel) : command === 'choice' ? '{"recorded":true}' : 'ok'
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('session.id', () => ({ value: 'session-1' }))
  on('session.cwd', () => ({ value: '/work/app' }))
  on('session.model', () => ({ value: 'claude-sonnet-5-5' }))
  on('session.surfaces', () => ({ value: ['terminal'] }))
  on('clock.now', () => ({ value: state.now }))
  on('turn.complete', ($, e) => ({ text: e.answer }))
  on('ui.status', () => ({ value: undefined }))
  on('env.set', ($, e) => {
    state.env = e.value
    return { value: undefined }
  })
  on('ui.open', ($, e) => {
    state.panes = [e.id]
    return { value: { isPlaced: true } }
  })
  on('ui.close', ($, e) => {
    state.panes = state.panes.filter(id => id !== e.id)
    return { value: undefined }
  })
  on('ui.panes', () => ({ value: state.panes.map(id => ({ id, title: 'System1Router', isShown: true, isFocused: true, isPlaced: true })) }))
  on('ui.toast', ($, e) => {
    state.toasts.push(e.text)
    return { value: undefined }
  })
  on('tool.call', { tool: 'AskUserQuestion' }, ($, e) => {
    if (state.answer === undefined) return { deny: 'dismissed' }
    const question = e.questions[0]?.question ?? ''
    return { result: { questions: e.questions, answers: { [question]: state.answer } } }
  })
  on('prompt.submit', ($, e) => {
    state.submitted.push(e.text)
    return { text: e.text }
  })
  on('turn.start', ($, e) => ({ turnId: e.turnId }))
  on('turn.step', async function* ($, e) {
    state.tried.push(e.model)
    // The API refuses an unknown id: the stream ends with no content and no stop reason.
    if (state.unknown.includes(e.model)) return { turnId: e.turnId, index: e.index, answer: '', toolUses: [], stopReason: null, usage: null }
    state.sent = { model: e.model, effort: e.effort }
    return { turnId: e.turnId, index: e.index, answer: 'ok', toolUses: [], stopReason: 'end_turn', usage: null }
  })
  return state
}

/** Submits a typed prompt and runs one model request of its turn; returns where it was sent. */
async function turn($: Engine, state: Fake, text = 'refactor the parser', turnId = 'turn-1', effort = 'medium'): Promise<Fake['sent']> {
  await $.prompt.submit({ text, origin: { kind: 'composer' }, wait: false })
  await $.turn.start({ text, turnId })
  for await (const _ of $.turn.step({ turnId, index: 0, model: 'claude-sonnet-5-5', effort: effort as 'medium', messageCount: 1 })) { /* drain */ }
  return state.sent
}

/** Ends a main-loop turn with what its requests cost. */
async function complete($: Engine, turnId: string, cacheRead: number, cacheWrite: number, isAborted = false): Promise<void> {
  await $.turn.complete({ turnId, answer: 'ok', durationMs: 1, isAborted, reason: isAborted ? 'aborted' : 'answer', usage: { model: 'claude-sonnet-5-5', input_tokens: 0, output_tokens: 10, cache_read_input_tokens: cacheRead, cache_creation_input_tokens: cacheWrite } })
}

test('an accepted suggestion sends the turn to that model and effort', async ($, on) => {
  const state = engine(on, ASK, 'Use opus · high')
  expect(await turn($, state)).toEqual({ model: 'claude-opus-5-5', effort: 'high' })
  expect(JSON.parse(state.stdin[0] ?? '{}')).toEqual({ session_id: 'session-1', cwd: '/work/app', model: 'claude-sonnet-5-5', prompt: 'refactor the parser', hook_event_name: 'UserPromptSubmit' })
  expect(state.calls).toContainEqual(['choice', '--current', 'claude-sonnet-5-5', '--suggested', 'opus', '--variant', 'high', '--accept', 'yes'])
})

test('always remembers the pair; keep and a dismissed dialog keep the model', async ($, on) => {
  const state = engine(on, ASK, 'Always switch to opus')
  expect((await turn($, state, 'refactor', 'turn-1')).model).toBe('claude-opus-5-5')
  expect(state.calls.find(call => call[0] === 'choice')).toContain('--remember')

  state.answer = 'Keep sonnet 5.5'
  expect((await turn($, state, 'refactor', 'turn-2')).model).toBe('claude-sonnet-5-5')
  expect(state.calls.filter(call => call[0] === 'choice').at(-1)).toContain('no')

  state.answer = undefined
  expect((await turn($, state, 'refactor', 'turn-3')).model).toBe('claude-sonnet-5-5')
  expect(state.calls.filter(call => call[0] === 'choice')).toHaveLength(3)
})

test('a remembered answer switches without asking', async ($, on) => {
  const state = engine(on, { ...ASK, action: 'switch', remembered: true }, undefined)
  expect((await turn($, state)).model).toBe('claude-opus-5-5')
})

test('an id the API refuses falls back to the next, then to the session model', async ($, on) => {
  const state = engine(on, { ...ASK, action: 'switch', remembered: true, modelIds: ['claude-opus-9-9', 'claude-opus-5-5'] })
  state.unknown = ['claude-opus-9-9']
  expect((await turn($, state, 'refactor', 'turn-1')).model).toBe('claude-opus-5-5')
  expect(state.tried).toEqual(['claude-opus-9-9', 'claude-opus-5-5'])

  state.tried = []
  expect((await turn($, state, 'refactor', 'turn-2')).model).toBe('claude-opus-5-5')
  expect(state.tried).toEqual(['claude-opus-5-5'])

  state.unknown = ['claude-opus-5-5']
  state.tried = []
  expect((await turn($, state, 'refactor', 'turn-3')).model).toBe('claude-sonnet-5-5')
  expect(state.tried).toEqual(['claude-opus-5-5', 'claude-sonnet-5-5'])
  expect(state.toasts.at(-1)).toContain('not available; this turn stays on sonnet 5.5')
})

test('slash commands and unrouted prompts leave the turn alone', async ($, on) => {
  const state = engine(on, { action: 'none', record: false, current: 'claude-sonnet-5-5', code: 'disabled' })
  expect((await turn($, state, '/model opus', 'turn-1')).model).toBe('claude-sonnet-5-5')
  expect(state.calls.some(call => call[0] === 'decide')).toBe(false)
  expect((await turn($, state, 'hello', 'turn-2')).model).toBe('claude-sonnet-5-5')
  expect(state.calls.filter(call => call[0] === 'decide')).toHaveLength(1)
})

test('the dashboard draws status, stats and remembered answers, and acts on them', async ($, on) => {
  const state = engine(on, ASK)
  await $.command.run({ command: 's1router', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
  for (const surface of ['terminal', 'desktop'] as const) {
    const ui = await $.ui.mount({
      plugin: 'system1router', surface, component: 'Pane', requestId: 's1router',
      props: { title: 'System1Router', isFocused: true, bodyColumns: 70, placement: 'dock', scroll: { offset: 0, bodyRows: 30 }, view: {} },
    })
    expect(await ui.find({ text: /Routing ON/ })).toBeDefined()
    expect(await ui.find({ text: /Esc closes/ })).toBeDefined()
    expect(await ui.find({ text: /anthropic\/claude-sonnet-5-5 → anthropic\/haiku/ })).toBeDefined()
    expect(await ui.find({ text: /4 answered · 3 accepted \(75%\)/ })).toBeDefined()
    await ui.press({ key: 'toggle' })
    expect(state.calls.at(-2)).toEqual(['toggle'])
    await ui.press({ key: 'forget-0' })
    expect(state.calls.at(-2)).toEqual(['forget', 'anthropic/claude-sonnet-5-5 → anthropic/haiku'])
    await ui.press({ key: 'close' })
    expect(state.panes).toEqual([])
    await ui.unmount()
    await $.command.run({ command: 's1router', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
  }
})

test('/s1router toggles the dashboard', async ($, on) => {
  const state = engine(on, ASK)
  const run = () => $.command.run({ command: 's1router', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: false, columns: 100 } })
  expect((await run()).text).toContain('opened')
  expect(state.panes).toEqual(['s1router'])
  expect((await run()).text).toContain('closed')
  expect(state.panes).toEqual([])
})

test('the mod marks its session for the command hook, again after /clear', async ($, on) => {
  const state = engine(on, ASK)
  on('command.list', () => ({ value: [] }))
  on('command.register', ($, e) => ({ value: { command: e.name } }))
  on('session.start', ($, e) => ({ cwd: e.cwd }))
  on('classic.SessionStart', () => ({}))
  await $.session.start({ cwd: '/work/app', surface: 'terminal', isInteractive: true })
  expect(state.env).toBe('session-1')
  await $.classic.SessionStart({ source: 'clear', session_id: 'session-2' })
  expect(state.env).toBe('session-2')
})

test('an unfocused pane on the terminal main screen says how to reach its buttons', async ($, on) => {
  engine(on, ASK)
  await $.command.run({ command: 's1router', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: false, columns: 100 } })
  const ui = await $.ui.mount({
    plugin: 'system1router', surface: 'terminal', component: 'Pane', requestId: 's1router',
    props: { title: 'System1Router', isFocused: false, bodyColumns: 90, placement: 'inline', scroll: { offset: 0, bodyRows: 20 }, view: {} },
    viewport: { columns: 100, rows: 40, isFullscreen: false },
  })
  expect(await ui.find({ text: /ctrl\+x tab to use the buttons/ })).toBeDefined()
  await ui.unmount()
})

test('an accepted effort keeps the model and holds for the following turns', async ($, on) => {
  const state = engine(on, EFFORT, 'Use xhigh effort')
  expect(await turn($, state, 'fix the race', 'turn-1')).toEqual({ model: 'claude-sonnet-5-5', effort: 'xhigh' })
  expect(state.calls).toContainEqual(['choice', '--current', 'claude-sonnet-5-5', '--suggested', 'claude-sonnet-5-5', '--variant', 'xhigh', '--accept', 'yes'])

  state.decision = UNCHANGED
  state.now = 5_000
  await complete($, 'turn-1', 0, 1_000)
  expect(await turn($, state, 'and the other one', 'turn-2')).toEqual({ model: 'claude-sonnet-5-5', effort: 'xhigh' })
  const stdin = JSON.parse(state.stdin.at(-1) ?? '{}')
  expect([stdin.effort, stdin.idle_ms]).toEqual(['xhigh', 0])

  // Subagents keep their own effort.
  for await (const _ of $.turn.step({ turnId: 'turn-2', index: 1, model: 'claude-sonnet-5-5', effort: 'medium', messageCount: 3, agentId: 'agent-1' })) { /* drain */ }
  expect(state.sent.effort).toBe('medium')
})

test('keeping the effort asks with the effort wording and changes nothing', async ($, on) => {
  const state = engine(on, { ...EFFORT, currentEffort: 'medium', correlationId: 's1r-1' }, 'Keep medium effort')
  expect(await turn($, state)).toEqual({ model: 'claude-sonnet-5-5', effort: 'medium' })
  expect(state.calls.find(call => call[0] === 'choice')).toEqual(['choice', '--current', 'claude-sonnet-5-5', '--current-variant', 'medium', '--suggested', 'claude-sonnet-5-5', '--variant', 'xhigh', '--accept', 'no', '--correlation', 's1r-1'])
})

test("the session's own /effort wins and pauses automatic effort", async ($, on) => {
  const state = engine(on, EFFORT, 'Use xhigh effort')
  state.panel = { ...PANEL, status: { ...PANEL.status, swap: 'effort' } }
  await $.command.run({ command: 's1router', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
  expect((await turn($, state, 'fix the race', 'turn-1')).effort).toBe('xhigh')
  // The person ran /effort high: the session's requests now name high.
  expect((await turn($, state, 'next', 'turn-2', 'high')).effort).toBe('high')
  expect(state.toasts.at(-1)).toContain('automatic effort is paused')
  expect(state.calls).toContainEqual(['override', '--session', 'session-1', '--model', 'claude-sonnet-5-5', '--effort', 'high', '--from', 'xhigh'])
  const decided = state.calls.filter(call => call[0] === 'decide').length
  expect((await turn($, state, 'again', 'turn-3', 'high')).effort).toBe('high')
  expect(state.calls.filter(call => call[0] === 'decide')).toHaveLength(decided)
})

test('the dashboard shows the effort, what a change did to the cache, and resumes', async ($, on) => {
  const state = engine(on, UNCHANGED)
  state.panel = { ...PANEL, status: { ...PANEL.status, swap: 'effort' }, stats: { ...PANEL.stats, efforts: { xhigh: 2 } } }
  await turn($, state, 'read the code', 'turn-1')
  await complete($, 'turn-1', 0, 1_000)
  state.decision = EFFORT
  state.answer = 'Use xhigh effort'
  await turn($, state, 'fix the race', 'turn-2')
  await complete($, 'turn-2', 100, 900)
  state.decision = UNCHANGED
  await turn($, state, 'and the test', 'turn-3')
  await complete($, 'turn-3', 950, 50)

  await $.command.run({ command: 's1router', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
  const ui = await $.ui.mount({
    plugin: 'system1router', surface: 'terminal', component: 'Pane', requestId: 's1router',
    props: { title: 'System1Router', isFocused: true, bodyColumns: 120, placement: 'dock', scroll: { offset: 0, bodyRows: 40 }, view: {} },
  })
  expect(await ui.find({ text: /effort swap/ })).toBeDefined()
  expect(await ui.find({ text: /now xhigh \(session medium\)/ })).toBeDefined()
  expect(await ui.find({ text: /low: question simple · xhigh: code hard/ })).toBeDefined()
  expect(await ui.find({ text: /after an effort change 10% \(1 turns\) · otherwise 95% \(1\)/ })).toBeDefined()
  expect(await ui.find({ text: /effort session → xhigh/ })).toBeDefined()
  expect(await ui.find({ text: /By effort: xhigh 2/ })).toBeDefined()
  await ui.press({ key: 'drop' })
  expect((await turn($, state, 'small fix', 'turn-4')).effort).toBe('medium')
  await ui.press({ key: 'pause' })
  const decided = state.calls.filter(call => call[0] === 'decide').length
  await turn($, state, 'paused', 'turn-5')
  expect(state.calls.filter(call => call[0] === 'decide')).toHaveLength(decided)
  await ui.unmount()
})

const MENU = { ...EFFORT, currentEffort: 'medium', style: 'menu', correlationId: 's1r-7' }

test('the menu applies the suggested effort, another picked one, or one typed under Other', async ($, on) => {
  const state = engine(on, MENU, 'xhigh (suggested)')
  expect((await turn($, state, 'fix the race', 'turn-1')).effort).toBe('xhigh')

  state.answer = 'high'
  expect((await turn($, state, 'fix the race', 'turn-2')).effort).toBe('high')
  expect(state.calls).toContainEqual(['override', '--session', 'session-1', '--model', 'claude-sonnet-5-5', '--effort', 'high', '--from', 'xhigh', '--correlation', 's1r-7'])
  expect(state.calls.filter(call => call[0] === 'choice').at(-1)).toContain('no')

  state.answer = 'max'
  expect((await turn($, state, 'fix the race', 'turn-3')).effort).toBe('max')
  state.answer = 'medium (current)'
  expect((await turn($, state, 'fix the race', 'turn-4')).effort).toBe('max')
})

test('a [high] tag sets that effort for its turn only and records it against the router', async ($, on) => {
  const state = engine(on, { ...UNCHANGED, currentEffort: 'medium', effort: undefined, correlationId: 's1r-9' })
  expect((await turn($, state, '[xhigh] fix it', 'turn-1')).effort).toBe('xhigh')
  expect(state.submitted.at(-1)).toBe('fix it')
  expect(JSON.parse(state.stdin.at(-1) ?? '{}').prompt).toBe('fix it')
  expect(state.calls).toContainEqual(['override', '--session', 'session-1', '--model', 'claude-sonnet-5-5', '--effort', 'xhigh', '--from', 'medium', '--correlation', 's1r-9'])
  await complete($, 'turn-1', 0, 100)
  expect((await turn($, state, 'thanks', 'turn-2')).effort).toBe('medium')
})

test('resending an aborted prompt with a tag corrects its decision without classifying again', async ($, on) => {
  const state = engine(on, { ...EFFORT, action: 'switch', variant: 'low', effort: 'low', correlationId: 's1r-3' })
  expect((await turn($, state, 'make the cache safe across processes', 'turn-1')).effort).toBe('low')
  await complete($, 'turn-1', 0, 100, true)
  const decided = state.calls.filter(call => call[0] === 'decide').length
  expect((await turn($, state, '[xhigh] make the cache safe across processes', 'turn-2')).effort).toBe('xhigh')
  expect(state.calls.filter(call => call[0] === 'decide')).toHaveLength(decided)
  expect(state.calls).toContainEqual(['override', '--session', 'session-1', '--model', 'claude-sonnet-5-5', '--effort', 'xhigh', '--from', 'low', '--correlation', 's1r-3'])
})
