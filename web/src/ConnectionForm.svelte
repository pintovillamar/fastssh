<script lang="ts">
  import Select from './Select.svelte'
  import {
    createConnection,
    deleteConnection,
    forgetHostKey,
    updateConnection,
    type AuthKind,
    type ConnectionDetails,
    type SavedConnection,
    type SecretChanges,
  } from './api'

  let {
    connection,
    ondone,
    oncancel,
  }: { connection: SavedConnection | null; ondone: () => void; oncancel: () => void } = $props()

  // The form edits a copy; the list only changes once the server accepts it.
  // svelte-ignore state_referenced_locally
  const initial = connection
  let name = $state(initial?.name ?? '')
  let host = $state(initial?.host ?? '')
  let port = $state(initial?.port ?? 22)
  let username = $state(initial?.username ?? '')
  let auth = $state<AuthKind>(initial?.auth ?? 'key')

  // Secrets are write-only: the server never sends them back, so each field
  // starts empty and "saved" only reflects what the server holds.
  let password = $state('')
  let privateKey = $state('')
  let keyPassphrase = $state('')
  let saved = $state({
    password: initial?.has_password ?? false,
    privateKey: initial?.has_private_key ?? false,
    keyPassphrase: initial?.has_key_passphrase ?? false,
  })
  // Ticked "remove" on a saved secret.
  let remove = $state({ password: false, keyPassphrase: false })

  let error = $state('')
  let notice = $state('')
  let busy = $state(false)
  let confirmingDelete = $state(false)
  let filePicker = $state<HTMLInputElement>()

  async function attempt(action: () => Promise<unknown>, after: () => void) {
    busy = true
    error = ''
    notice = ''
    try {
      await action()
      after()
    } catch (err) {
      error = (err as Error).message
    } finally {
      busy = false
    }
  }

  function save(event: SubmitEvent) {
    event.preventDefault()
    if (auth === 'key' && !privateKey.trim() && !saved.privateKey) {
      error = 'Add the private key for this connection.'
      return
    }
    const details: ConnectionDetails = { name, host, port, username, auth }
    const secrets: SecretChanges = {}
    if (password) secrets.password = password
    else if (remove.password) secrets.password = null
    if (privateKey.trim()) secrets.private_key = privateKey
    if (keyPassphrase) secrets.key_passphrase = keyPassphrase
    else if (remove.keyPassphrase) secrets.key_passphrase = null
    attempt(
      () => (initial ? updateConnection(initial.id, details, secrets) : createConnection(details, secrets)),
      ondone,
    )
  }

  async function readKeyFile() {
    const file = filePicker?.files?.[0]
    if (!file || !filePicker) return
    // Real private keys are a few kilobytes; anything large is a wrong pick.
    if (file.size > 64 * 1024) {
      error = 'That file is too large to be a private key.'
    } else {
      privateKey = await file.text()
      error = ''
    }
    filePicker.value = ''
  }

  function removeConnection() {
    if (!initial) return
    attempt(() => deleteConnection(initial.id), ondone)
  }

  function forget() {
    if (!initial) return
    attempt(
      () => forgetHostKey(initial.id),
      () => (notice = 'Host key forgotten. You will be asked to trust it on the next connect.'),
    )
  }
</script>

<form onsubmit={save}>
  <h1>{initial ? 'Edit connection' : 'New connection'}</h1>

  <div class="pair">
    <label class="grow">
      Host
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={host} required autofocus autocapitalize="none" autocorrect="off" spellcheck="false" placeholder="example.com" />
    </label>
    <label class="port">
      Port
      <input type="number" bind:value={port} required min="1" max="65535" inputmode="numeric" />
    </label>
  </div>

  <label>
    Username
    <input bind:value={username} required autocapitalize="none" autocorrect="off" spellcheck="false" autocomplete="off" />
  </label>

  <label>
    Name
    <input bind:value={name} placeholder="Optional — defaults to user@host" autocomplete="off" />
  </label>

  <div class="field">
    <span id="auth-kind-label">Sign in with</span>
    <Select
      bind:value={auth}
      labelledby="auth-kind-label"
      options={[
        { value: 'key', label: 'Private key' },
        { value: 'password', label: 'Password' },
      ]}
    />
  </div>

  {#if auth === 'password'}
    <label>
      Password
      <input
        type="password"
        bind:value={password}
        autocomplete="new-password"
        disabled={remove.password}
        placeholder={saved.password ? 'Saved — leave empty to keep it' : 'Optional — leave empty to be asked each time'}
      />
    </label>
    {#if saved.password}
      <label class="check">
        <input type="checkbox" bind:checked={remove.password} />
        Remove the saved password
      </label>
    {/if}
  {:else}
    <div class="field">
      <div class="field-head">
        <span id="key-label">Private key</span>
        <button type="button" class="plain small" onclick={() => filePicker?.click()}>Choose file…</button>
        <input type="file" hidden bind:this={filePicker} onchange={readKeyFile} />
      </div>
      <textarea
        bind:value={privateKey}
        aria-labelledby="key-label"
        rows="5"
        autocapitalize="none"
        autocomplete="off"
        spellcheck="false"
        placeholder={saved.privateKey
          ? 'Saved — paste another key to replace it'
          : 'Paste the private key: the file without .pub, or a PuTTY .ppk file'}
      ></textarea>
    </div>
    <label>
      Key passphrase
      <input
        type="password"
        bind:value={keyPassphrase}
        autocomplete="new-password"
        disabled={remove.keyPassphrase}
        placeholder={saved.keyPassphrase ? 'Saved — leave empty to keep it' : 'Optional — leave empty to be asked each time'}
      />
    </label>
    {#if saved.keyPassphrase}
      <label class="check">
        <input type="checkbox" bind:checked={remove.keyPassphrase} />
        Remove the saved passphrase
      </label>
    {/if}
  {/if}
  <p class="help">Anything you save here is encrypted in your vault.</p>

  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if notice}<p class="help">{notice}</p>{/if}

  <div class="actions">
    <button type="submit" class="primary" disabled={busy}>Save</button>
    <button type="button" onclick={oncancel} disabled={busy}>Cancel</button>
    {#if initial}
      <span class="spacer"></span>
      {#if confirmingDelete}
        <button type="button" class="danger" onclick={removeConnection} disabled={busy}>Delete permanently</button>
        <button type="button" onclick={() => (confirmingDelete = false)} disabled={busy}>Keep</button>
      {:else}
        <button type="button" onclick={forget} disabled={busy}>Forget host key</button>
        <button type="button" class="danger" onclick={() => (confirmingDelete = true)} disabled={busy}>Delete</button>
      {/if}
    {/if}
  </div>
</form>

<style>
  form {
    display: grid;
    gap: 16px;
  }

  h1 {
    font-size: 20px;
    font-weight: 650;
    margin: 0 0 4px;
  }

  .pair {
    display: flex;
    gap: 12px;
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .port {
    width: 96px;
  }

  .field-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .small {
    font-size: 13px;
    padding: 2px 8px;
    color: var(--accent);
  }

  textarea {
    font-family: ui-monospace, 'JetBrains Mono', Menlo, monospace;
    font-size: 12px;
    line-height: 1.4;
    color: var(--text);
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: -8px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 4px;
  }

  .spacer {
    flex: 1;
  }
</style>
