export type AuthKind = 'password' | 'key'

export interface ConnectionDetails {
  name: string
  host: string
  port: number
  username: string
  auth: AuthKind
}

export interface SavedConnection extends ConnectionDetails {
  id: number
  // Secrets never come back from the server, only whether they are saved.
  has_password: boolean
  has_private_key: boolean
  has_key_passphrase: boolean
}

/** Per secret: leave out to keep what is saved, `null` to remove, text to replace. */
export interface SecretChanges {
  password?: string | null
  private_key?: string | null
  key_passphrase?: string | null
}

export interface Session {
  state: 'setup' | 'signed_out' | 'new_vault' | 'locked' | 'ready'
  email: string | null
  /** Whether the vault passphrase is the account's login password. */
  has_password: boolean
  google: boolean
  signup: boolean
  local_shell: boolean
}

/** Called when the server says the session ended or the vault locked. */
let sessionLost = () => {}
export function onSessionLost(handler: () => void) {
  sessionLost = handler
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api${path}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (!response.ok) {
    // On the sign-in endpoints these statuses just mean a wrong password.
    if ((response.status === 401 || response.status === 423) && path.startsWith('/connections')) {
      sessionLost()
    }
    const problem = await response.json().catch(() => null)
    throw new Error(problem?.error ?? `Request failed (${response.status})`)
  }
  const text = await response.text()
  return (text ? JSON.parse(text) : undefined) as T
}

export const getSession = () => request<Session>('GET', '/session')
export const signUp = (email: string, password: string) => request<void>('POST', '/signup', { email, password })
export const signIn = (email: string, password: string) => request<void>('POST', '/login', { email, password })
export const signOut = () => request<void>('POST', '/logout')
export const unlock = (passphrase: string) => request<void>('POST', '/unlock', { passphrase })

export const listConnections = () => request<SavedConnection[]>('GET', '/connections')
export const createConnection = (details: ConnectionDetails, secrets: SecretChanges) =>
  request<SavedConnection>('POST', '/connections', { ...details, secrets })
export const updateConnection = (id: number, details: ConnectionDetails, secrets: SecretChanges) =>
  request<SavedConnection>('PUT', `/connections/${id}`, { ...details, secrets })
export const deleteConnection = (id: number) => request<void>('DELETE', `/connections/${id}`)
export const forgetHostKey = (id: number) => request<void>('DELETE', `/connections/${id}/host-key`)
