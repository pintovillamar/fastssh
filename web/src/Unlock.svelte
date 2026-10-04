<script lang="ts">
  import { signOut, unlock, type Session } from './api'

  let { session, ondone }: { session: Session; ondone: () => void } = $props()

  // svelte-ignore state_referenced_locally
  const creating = session.state === 'new_vault'
  // svelte-ignore state_referenced_locally
  const word = session.desktop ? 'master password' : session.has_password ? 'password' : 'vault passphrase'
  let passphrase = $state('')
  let confirm = $state('')
  let error = $state('')
  let busy = $state(false)

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (creating && passphrase !== confirm) {
      error = 'The two passphrases do not match.'
      return
    }
    busy = true
    error = ''
    try {
      await unlock(passphrase)
      ondone()
    } catch (err) {
      error = (err as Error).message
    } finally {
      busy = false
    }
  }

  async function leave() {
    await signOut().catch(() => {})
    ondone()
  }
</script>

<div class="screen">
  <form onsubmit={submit}>
    <div class="brand">FastSSH</div>
    <h1>{creating ? 'Create a vault passphrase' : session.desktop ? 'Unlock FastSSH' : 'Unlock your vault'}</h1>
    <p class="help">
      {#if creating}
        Your saved passwords and keys are encrypted with this passphrase. Google signs you in, but
        only the passphrase can unlock them. If you forget it, they cannot be recovered.
      {:else if session.desktop}
        Enter your {word} to unlock your saved passwords and keys.
      {:else}
        Signed in as {session.email}. Enter your {word} to unlock your saved passwords and keys.
      {/if}
    </p>

    <!-- Lets password managers file the passphrase under the right account. -->
    <input type="email" value={session.email} autocomplete="username" hidden readonly />
    <label>
      {creating ? 'Vault passphrase' : word[0].toUpperCase() + word.slice(1)}
      <!-- svelte-ignore a11y_autofocus -->
      <input type="password" bind:value={passphrase} required autofocus minlength={creating ? 8 : undefined} autocomplete={creating ? 'new-password' : 'current-password'} />
    </label>
    {#if creating}
      <label>
        Repeat passphrase
        <input type="password" bind:value={confirm} required autocomplete="new-password" />
      </label>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <button type="submit" class="primary" disabled={busy}>{creating ? 'Create vault' : 'Unlock'}</button>
    {#if !session.desktop}
      <button type="button" class="plain switch" onclick={leave}>Sign out</button>
    {/if}
  </form>
</div>
