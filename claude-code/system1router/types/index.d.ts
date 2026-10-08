/** Reasoning effort a router variant maps to. */
export type Effort = 'low' | 'medium' | 'high' | 'xhigh' | 'max'

/** One routed prompt of this session. */
export type Routed = {
  routingClass: string
  confidence: number
  current: string
  model: string
  effort: Effort | null
  /** The turn ran on the suggested model, or kept the current one. */
  outcome: 'switched' | 'kept'
  /** The dialog, a remembered answer, the router's consent mode, or a dismissed dialog. */
  how: 'asked' | 'remembered' | 'auto' | 'dismissed'
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
  }
  preferences: { key: string; accept: boolean; current: string; suggested: string; createdAt: number }[]
}

declare module 'claude-code' {
  interface PluginState {
    system1router: {
      routed: Routed[]
      panel: Panel | null
      problem: string | null
    }
  }
}
