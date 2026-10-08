import { expect, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'
import type { On } from 'claude-code'

const PANEL = {
  status: { enabled: true, provider: 'jev', providerReady: true, rules: 12, claudeCodeRules: 6, consentMode: 'interactive', configPath: '/c/config.json', providerLog: '/s/provider-debug.jsonl' },
  stats: { calls: 10, models: { 'anthropic/claude-sonnet-5-5': 7, 'anthropic/opus': 3 }, answered: 4, accepted: 3, advised: 1, byHost: { 'claude-code': 10 } },
  preferences: [{ key: 'anthropic/claude-sonnet-5-5 → anthropic/haiku', accept: false, current: 'anthropic/claude-sonnet-5-5', suggested: 'anthropic/haiku', createdAt: 1 }],
}

const ASK = {
  action: 'ask', record: true, current: 'claude-sonnet-5-5', model: 'opus', modelIds: ['claude-opus-5-5'], variant: 'high', remembered: null,
  routingClass: 'code_hard', confidence: 0.8, reason: 'Complex or high-risk change', code: 'consent_requested',
}

type Fake = {
  decision: object
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
}

/** The engine beneath the mod: the binary, the session, the dialog and the turn. */
function engine(on: On, decision: object, answer?: string): Fake {
  const state: Fake = { decision, answer, calls: [], stdin: [], sent: { model: '', effort: undefined }, tried: [], unknown: [], toasts: [], env: undefined }
  on('process.run', ($, e) => {
    state.calls.push([...e.argv.slice(1)])
    if (e.init?.stdin) state.stdin.push(e.init.stdin)
    const command = e.argv[1]
    const stdout = command === 'decide' ? JSON.stringify(state.decision) : command === 'panel' ? JSON.stringify(PANEL) : command === 'choice' ? '{"recorded":true}' : 'ok'
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('session.id', () => ({ value: 'session-1' }))
  on('session.cwd', () => ({ value: '/work/app' }))
  on('session.model', () => ({ value: 'claude-sonnet-5-5' }))
  on('session.surfaces', () => ({ value: ['terminal'] }))
  on('ui.status', () => ({ value: undefined }))
  on('env.set', ($, e) => {
    state.env = e.value
    return { value: undefined }
  })
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('ui.toast', ($, e) => {
    state.toasts.push(e.text)
    return { value: undefined }
  })
  on('tool.call', { tool: 'AskUserQuestion' }, ($, e) => {
    if (state.answer === undefined) return { deny: 'dismissed' }
    const question = e.questions[0]?.question ?? ''
    return { result: { questions: e.questions, answers: { [question]: state.answer } } }
  })
  on('prompt.submit', ($, e) => ({ text: e.text }))
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
async function turn($: Engine, state: Fake, text = 'refactor the parser', turnId = 'turn-1'): Promise<Fake['sent']> {
  await $.prompt.submit({ text, origin: { kind: 'composer' }, wait: false })
  await $.turn.start({ text, turnId })
  for await (const _ of $.turn.step({ turnId, index: 0, model: 'claude-sonnet-5-5', effort: 'medium', messageCount: 1 })) { /* drain */ }
  return state.sent
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
    expect(await ui.find({ text: /anthropic\/claude-sonnet-5-5 → anthropic\/haiku/ })).toBeDefined()
    expect(await ui.find({ text: /4 answered · 3 accepted \(75%\)/ })).toBeDefined()
    await ui.press({ key: 'toggle' })
    expect(state.calls.at(-2)).toEqual(['toggle'])
    await ui.press({ key: 'forget-0' })
    expect(state.calls.at(-2)).toEqual(['forget', 'anthropic/claude-sonnet-5-5 → anthropic/haiku'])
    await ui.unmount()
  }
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
