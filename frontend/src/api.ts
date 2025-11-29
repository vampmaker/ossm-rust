import type { MotorControllerConfig, PausedControlPayload, MotorState } from './types'

const api_base = window.location.hostname === 'localhost' ? 'http://ossm.lan' : document.location.href;

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(new URL(path, api_base).toString())
  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`)
  }
  return await response.json()
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(new URL(path, api_base).toString(), {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`)
  }
  return await response.json()
}

export async function getConfig(): Promise<MotorControllerConfig> {
  return getJson<MotorControllerConfig>('/config')
}

export async function setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
  return postJson<MotorControllerConfig>('/config', config)
}

export async function setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
  return postJson<MotorControllerConfig>('/paused', payload)
}

export async function getState(): Promise<MotorState> {
  return getJson<MotorState>('/state')
}
