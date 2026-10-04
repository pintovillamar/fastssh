export type AuthMethod = { kind: 'password' } | { kind: 'key_file'; path: string }

export interface ConnectionDetails {
  name: string
  host: string
  port: number
  username: string
  auth: AuthMethod
}

export interface SavedConnection extends ConnectionDetails {
  id: number
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api${path}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (!response.ok) {
    const problem = await response.json().catch(() => null)
    throw new Error(problem?.error ?? `Request failed (${response.status})`)
  }
  return response.status === 204 ? (undefined as T) : response.json()
}

export const listConnections = () => request<SavedConnection[]>('GET', '/connections')
export const createConnection = (details: ConnectionDetails) =>
  request<SavedConnection>('POST', '/connections', details)
export const updateConnection = (id: number, details: ConnectionDetails) =>
  request<SavedConnection>('PUT', `/connections/${id}`, details)
export const deleteConnection = (id: number) => request<void>('DELETE', `/connections/${id}`)
export const forgetHostKey = (id: number) => request<void>('DELETE', `/connections/${id}/host-key`)
