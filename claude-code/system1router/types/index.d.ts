/** Reasoning effort a router variant maps to. */
export type Effort = 'low' | 'medium' | 'high' | 'xhigh' | 'max'

/** One routed prompt of this session. */
export type Routed = {
  /** `effort` keeps the model and changes its effort; `model` switches models. */
  kind: 'model' | 'effort'
  routingClass: string
  confidence: number
  current: string
  model: string
  effort: Effort | null
  /** The effort before an effort swap, when known. */
  from: Effort | null
  /** The turn ran on the suggested model, or kept the current one. */
  outcome: 'switched' | 'kept'
  /** The dialog, a remembered answer, the router's consent mode, or a dismissed dialog. */
  how: 'asked' | 'remembered' | 'auto' | 'dismissed'
}

/** The effort the router holds for the main loop of this session. */
export type EffortState = {
  /** Sent instead of the session's effort until the router picks another. */
  applied: Effort | null
  /** The session's own effort, as its last model request named it. */
  base: Effort | null
  /** Automatic effort is off for this session (the pane, or a manual /effort). */
  paused: boolean
}

/** What one main-loop turn cost, to show what an effort change does to the cache. */
export type TurnCost = {
  effort: Effort | null
  /** The turn ran at another effort than the turn before it. */
  switched: boolean
  cacheRead: number
  cacheWrite: number
  input: number
  output: number
}

/** `system1router panel`: status, stats across hosts and remembered answers. */
export type Panel = {
  status: {
    enabled: boolean
    provider: string
    providerReady: boolean
    rules: number
    claudeCodeRules: number
    consentMode: string
    /** What a decision changes (`routing.swap`). */
    swap: 'model' | 'effort'
    /** Effort per routing class (`effort.levels` and defaults). */
    efforts: Record<string, string>
    configPath: string | null
    providerLog: string | null
  }
  stats: {
    calls: number
    models: Record<string, number>
    answered: number
    accepted: number
    advised: number
    byHost: Record<string, number>
    efforts: Record<string, number>
  }
  preferences: { key: string; accept: boolean; current: string; suggested: string; createdAt: number }[]
}

declare module 'claude-code' {
  interface PluginState {
    system1router: {
      routed: Routed[]
      panel: Panel | null
      problem: string | null
      effort: EffortState
      costs: TurnCost[]
    }
  }
}
