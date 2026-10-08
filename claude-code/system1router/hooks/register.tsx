import { atom, read, update } from 'claude-code'
import type { EngineInterface, PromptOrigin, Register, TurnStepChunk } from 'claude-code'

import type { Effort, Panel, Routed } from '../types'

// System1Router for Claude Code. The native `system1router` binary classifies
// each typed prompt (`decide`); when it suggests another model the person
// chooses in a native dialog, and every model request of that turn is sent to
// the chosen model (`turn.step`). The session's own model is never changed.

const PANE = 's1router'
const routed = atom({ plugin: 'system1router', key: 'routed' } as const, [])
const panel = atom({ plugin: 'system1router', key: 'panel' } as const, null)
const problem = atom({ plugin: 'system1router', key: 'problem' } as const, null)

const EFFORTS: readonly string[] = ['low', 'medium', 'high', 'xhigh', 'max']
const HISTORY = 30

/** What `system1router decide` answers (see `claude_code::mod_decision`). */
type Decision = {
  action: 'ask' | 'switch' | 'keep' | 'none'
  record: boolean
  current?: string | null
  model?: string
  /** Ids the API knows for `model`, best first (`claude_code::model_ids`). */
  modelIds?: string[]
  variant?: string | null
  remembered?: boolean | null
  routingClass?: string
  confidence?: number
  reason?: string
  code?: string
}

type Choice = { model: string; ids: string[]; current: string; effort: Effort | null }

/** `claude-sonnet-5-5` reads `sonnet 5.5`; any other id as it is. */
export function short(model: string): string {
  const match = /^claude-([a-z]+)-(\d+)-(\d+)/.exec(model)
  return match ? `${match[1]} ${match[2]}.${match[3]}` : model
}

/** The router's own rule: a target `opus` matches `claude-opus-5-5`. */
export function sameModel(current: string, target: string): boolean {
  return current === target || current.split(/[-.[]/).includes(target)
}

/** Prompts a person wrote; notifications, peers and plugins are not routed. */
function typed(origin: PromptOrigin): boolean {
  return origin.kind === 'composer' || origin.kind === 'bridge' || origin.kind === 'sdk' || (origin.kind === 'plugin' && origin.asUser === true)
}

function idle(decision: Decision): string {
  if (decision.code === 'disabled') return 'S1R off'
  const current = decision.current ? short(decision.current) : 'current model'
  if (decision.routingClass) return `S1R · ${decision.routingClass}${decision.code === 'cache_sticky' ? ' · cache kept' : ''} · ${current}`
  return 'S1R on'
}

// The binary the mod calls: the `executable` option, set as the module loads.
let executable = 'system1router'

async function run($: EngineInterface, args: readonly string[], stdin?: string): Promise<string> {
  const { exitCode, stdout, stderr } = await $.process.run([executable, ...args], { stdin, timeoutMs: 15_000 })
  if (exitCode !== 0) throw new Error(stderr.trim() || `${executable} ${args[0]} exited with ${exitCode}`)
  return stdout.trim()
}

async function refresh($: EngineInterface): Promise<Panel | null> {
  try {
    const data = JSON.parse(await run($, ['panel'])) as Panel
    await update($, panel, () => data)
    await update($, problem, () => null)
    return data
  } catch (error) {
    await update($, problem, () => `${executable} panel failed: ${String(error)}`)
    return null
  }
}

async function openPane($: EngineInterface): Promise<void> {
  await refresh($)
  await $.ui.open({ id: PANE, title: 'System1Router' })
}

async function toggle($: EngineInterface, force?: 'on' | 'off'): Promise<string> {
  const text = await run($, force ? ['toggle', force] : ['toggle'])
  const data = await refresh($)
  if (data) $.ui.status(data.status.enabled ? 'S1R on' : 'S1R off')
  return text
}

async function forget($: EngineInterface, key?: string): Promise<void> {
  await run($, key ? ['forget', key] : ['forget'])
  await refresh($)
}

/** Asks when needed and returns the model the turn should use, if another. */
async function settle($: EngineInterface, decision: Decision): Promise<Choice | undefined> {
  const { model, current } = decision
  if (decision.action === 'none' || !model || !current) {
    $.ui.status(idle(decision))
    return undefined
  }
  const effort = decision.variant && EFFORTS.includes(decision.variant) ? (decision.variant as Effort) : null
  const target = effort ? `${model} · ${effort}` : model
  let accept = decision.action === 'switch'
  let remember = false
  let how: Routed['how'] = decision.remembered == null ? 'auto' : 'remembered'
  if (decision.action === 'ask') {
    // Nobody to ask (a -p run): keep the current model and record nothing.
    if ((await $.session.surfaces()).length === 0) {
      $.ui.status(idle(decision))
      return undefined
    }
    const labels = [`Use ${target}`, `Keep ${short(current)}`, `Always switch to ${model}`, `Never switch to ${model}`]
    const percent = Math.round((decision.confidence ?? 0) * 100)
    const reason = (decision.reason ?? '').replace(/\.\s*$/, '')
    try {
      const answer = await $.ui.ask(
        `S1R · ${decision.routingClass} (${percent}%)${reason ? `: ${reason}` : ''}. Use ${target} for this turn instead of ${short(current)}?`,
        { header: 'S1R', options: labels },
      )
      accept = answer === labels[0] || answer === labels[2]
      remember = answer === labels[2] || answer === labels[3]
      how = 'asked'
    } catch {
      // Escape keeps the current model, as in OpenCode.
      how = 'dismissed'
    }
  }
  if (decision.record) {
    const variant = decision.variant ? ['--variant', decision.variant] : []
    await run($, ['choice', '--current', current, '--suggested', model, ...variant, '--accept', accept ? 'yes' : 'no', ...(remember ? ['--remember'] : [])])
  }
  const entry: Routed = { routingClass: decision.routingClass ?? 'unknown', confidence: decision.confidence ?? 0, current, model, effort, outcome: accept ? 'switched' : 'kept', how }
  await update($, routed, list => [...list, entry].slice(-HISTORY))
  $.ui.status(accept ? `S1R · ${entry.routingClass} → ${target}` : `S1R · ${entry.routingClass} · kept ${short(current)}`)
  return accept ? { model, ids: decision.modelIds?.length ? decision.modelIds : [model], current, effort } : undefined
}

export const register: Register = (on, options) => {
  executable = typeof options.executable === 'string' && options.executable.trim() ? options.executable.trim() : 'system1router'
  // The choice for the turn the submitted prompt starts, then per turn. A
  // reload forgets them, which only means one turn keeps the session's model.
  let pending: Choice | undefined
  const turns = new Map<string, Choice>()
  // Ids the API refused in this process, skipped from then on.
  const rejected = new Set<string>()

  on('session.start', async ($, e, next) => {
    // The command hook of this session, if installed too, leaves routing to the mod.
    await $.env.set('SYSTEM1ROUTER_CLAUDE_MOD', await $.session.id())
    const known = new Set((await $.command.list()).map(command => command.name))
    await $.command.register({ name: 's1router', description: 'Open the System1Router dashboard' })
    // The hook installer writes these as command files; the mod answers them either way.
    if (!known.has('s1router_toggle')) await $.command.register({ name: 's1router_toggle', description: 'Toggle System1Router routing on/off', argumentHint: '[on|off]' })
    if (!known.has('s1router_config')) await $.command.register({ name: 's1router_config', description: 'Show System1Router status, config and log paths' })
    if (!known.has('s1router_stats')) await $.command.register({ name: 's1router_stats', description: 'Show System1Router calls per model and suggestion acceptance' })
    const data = await refresh($)
    $.ui.status(data ? (data.status.enabled ? 'S1R on' : 'S1R off') : 'S1R unavailable')
    return next(e)
  })

  // /clear and a resume give the session a new id, without a session.start.
  on('classic.SessionStart', async ($, e, next) => {
    await $.env.set('SYSTEM1ROUTER_CLAUDE_MOD', e.session_id)
    return next(e)
  })

  on('prompt.submit', async ($, e, next) => {
    pending = undefined
    if (!typed(e.origin) || e.turnId !== undefined || !e.text.trim() || e.text.trimStart().startsWith('/')) return next(e)
    $.ui.status('S1R · classifying…')
    try {
      const [session_id, cwd, model] = await Promise.all([$.session.id(), $.session.cwd(), $.session.model()])
      const stdin = JSON.stringify({ session_id, cwd, model, prompt: e.text, hook_event_name: 'UserPromptSubmit' })
      pending = await settle($, JSON.parse(await run($, ['decide'], stdin)) as Decision)
    } catch (error) {
      // Fail open: the prompt goes on with the session's model.
      $.ui.status('S1R unavailable')
      await update($, problem, () => String(error))
    }
    return next(e)
  })

  on('turn.start', ($, e, next) => {
    if (pending) turns.set(e.turnId, pending)
    pending = undefined
    return next(e)
  })

  on('turn.step', async function* ($, e, next) {
    // Subagents keep their own models.
    const choice = e.agentId === undefined ? turns.get(e.turnId) : undefined
    if (!choice || sameModel(e.model, choice.model)) return yield* next(e)
    for (const id of choice.ids.filter(id => !rejected.has(id))) {
      // An id the API does not know ends the stream with nothing but engine
      // items, so they are held until the response shows content.
      const held: TurnStepChunk[] = []
      let isLive = false
      const stream = next({ ...e, model: id, ...(choice.effort ? { effort: choice.effort } : {}) })
      while (true) {
        const item = await stream.next()
        if (item.done) {
          if (isLive || item.value?.stopReason != null) {
            yield* held
            return item.value
          }
          break
        }
        if (!isLive && item.value.kind === 'engine') {
          held.push(item.value)
          continue
        }
        if (!isLive) yield* held
        isLive = true
        yield item.value
      }
      rejected.add(id)
    }
    $.ui.toast(`S1R: ${choice.ids.join(', ')} not available; this turn stays on ${short(choice.current)}`)
    $.ui.status(`S1R · ${choice.model} unavailable · ${short(choice.current)}`)
    turns.delete(e.turnId)
    return yield* next(e)
  })

  on('turn.complete', ($, e, next) => {
    turns.delete(e.turnId)
    return next(e)
  })

  on('command.run', { command: 's1router' }, async $ => {
    await openPane($)
    return { text: 'System1Router dashboard opened.' }
  })

  on('command.run', { command: 's1router_toggle' }, async ($, e) => {
    const word = e.args.trim()
    return { text: await toggle($, word === 'on' || word === 'off' ? word : undefined) }
  })

  on('command.run', { command: 's1router_config' }, async $ => ({ text: await run($, ['status']) }))

  on('command.run', { command: 's1router_stats' }, async $ => ({ text: await run($, ['stats']) }))

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const data = await read($, panel)
    const list = await read($, routed)
    const failure = await read($, problem)
    const width = Math.max(30, e.props.bodyColumns ?? 60)

    if (!data) {
      return (
        <Box flexDirection="column">
          <Text color="red">{failure ?? 'No data from system1router yet.'}</Text>
          <Button key="refresh" label="Refresh" onPress={() => refresh($)} />
        </Box>
      )
    }

    const { status, stats } = data
    const rate = stats.answered > 0 ? `${Math.round((100 * stats.accepted) / stats.answered)}%` : '—'
    const models = Object.entries(stats.models).sort((a, b) => b[1] - a[1]).slice(0, 6)
    const top = models[0]?.[1] ?? 1
    const nameWidth = Math.min(32, Math.max(...models.map(([name]) => name.length), 8))
    const barWidth = Math.max(4, width - nameWidth - 10)

    return (
      <Box flexDirection="column">
        <Box flexDirection="row">
          <Text bold color={status.enabled ? 'green' : 'yellow'}>Routing {status.enabled ? 'ON' : 'OFF'} </Text>
          <Button key="toggle" hotkey="t" variant="primary" label={status.enabled ? 'Turn off' : 'Turn on'} onPress={() => toggle($)} />
          <Text> </Text>
          <Button key="refresh" hotkey="r" label="Refresh" onPress={() => refresh($)} />
        </Box>
        <Text dimColor>
          {status.provider}{status.providerReady ? '' : ' (not configured)'} · {status.rules} rules ({status.claudeCodeRules} for Claude Code) · consent {status.consentMode}
        </Text>
        {failure && <Text color="red">{failure}</Text>}

        <Box marginTop={1} flexDirection="column">
          <Text bold>This session</Text>
          {list.length === 0 && <Text dimColor>  No routed prompt yet.</Text>}
          {[...list].reverse().slice(0, 8).map(item => (
            <Text>
              {'  '}
              <Text color={item.outcome === 'switched' ? 'green' : undefined}>{item.outcome === 'switched' ? '→' : '·'}</Text>
              {` ${item.routingClass} ${Math.round(item.confidence * 100)}%  ${short(item.current)} → ${item.model}${item.effort ? ` · ${item.effort}` : ''}  `}
              <Text dimColor>{item.outcome} ({item.how})</Text>
            </Text>
          ))}
        </Box>

        <Box marginTop={1} flexDirection="column">
          <Text bold>All hosts · {stats.calls} prompts</Text>
          <Text dimColor>  {stats.answered} answered · {stats.accepted} accepted ({rate}) · {stats.advised} advisory</Text>
          {models.map(([name, count]) => (
            <Text>
              {'  '}{name.length > nameWidth ? `${name.slice(0, nameWidth - 1)}…` : name.padEnd(nameWidth)}{' '}
              <Text color="cyan">{'█'.repeat(Math.max(1, Math.round((barWidth * count) / top)))}</Text> {count}
            </Text>
          ))}
        </Box>

        <Box marginTop={1} flexDirection="column">
          <Text bold>Remembered answers</Text>
          {data.preferences.length === 0 && <Text dimColor>  None. Choose Always or Never in the dialog to add one.</Text>}
          {data.preferences.map((item, index) => (
            <Box flexDirection="row">
              <Text>
                {'  '}
                <Text color={item.accept ? 'green' : 'yellow'}>{item.accept ? 'always' : 'never '}</Text> {item.current} → {item.suggested}{' '}
              </Text>
              <Button key={`forget-${index}`} label="Forget" onPress={() => forget($, item.key)} />
            </Box>
          ))}
          {data.preferences.length > 1 && <Button key="forget-all" label="Forget all" onPress={() => forget($)} />}
        </Box>

        <Box marginTop={1} flexDirection="column">
          <Text dimColor>Config {status.configPath ?? '(none)'}</Text>
          <Text dimColor>Provider log {status.providerLog ?? '(none)'}</Text>
        </Box>
      </Box>
    )
  })
}
