<script lang="ts">
  import { signIn, signUp, type Session } from './api'

  let { session, ondone }: { session: Session; ondone: () => void } = $props()

  // svelte-ignore state_referenced_locally
  const setup = session.state === 'setup'
  // svelte-ignore state_referenced_locally
  const desktop = session.desktop
  let creating = $state(setup)
  let email = $state('')
  let password = $state('')
  let confirm = $state('')
  let busy = $state(false)

  // Google sign-in reports problems by sending the browser back with ?error=.
  const params = new URLSearchParams(location.search)
  let error = $state(params.get('error') ?? '')
  if (params.has('error')) history.replaceState(null, '', location.pathname)

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (creating && password !== confirm) {
      error = 'The two passwords do not match.'
      return
    }
    busy = true
    error = ''
    try {
      await (creating ? signUp(email, password) : signIn(email, password))
      ondone()
    } catch (err) {
      error = (err as Error).message
    } finally {
      busy = false
    }
  }

  function toggle() {
    creating = !creating
    error = ''
    confirm = ''
  }
</script>

<div class="screen">
  <form onsubmit={submit}>
    <div class="brand">FastSSH</div>
    {#if desktop}
      <h1>{setup ? 'Create a master password' : 'Unlock FastSSH'}</h1>
    {:else}
      <h1>{setup ? 'Create the first account' : creating ? 'Create an account' : 'Sign in'}</h1>
    {/if}
    {#if setup && !desktop}
      <p class="help">This account will be the admin of this FastSSH server.</p>
    {/if}

    {#if !desktop}
      <label>
        Email
        <!-- svelte-ignore a11y_autofocus -->
        <input type="email" bind:value={email} required autofocus autocomplete="username" autocapitalize="none" spellcheck="false" />
      </label>
    {/if}
    <label>
      {desktop ? 'Master password' : 'Password'}
      <!-- svelte-ignore a11y_autofocus -->
      <input type="password" bind:value={password} required autofocus={desktop} minlength={creating ? 8 : undefined} autocomplete={creating ? 'new-password' : 'current-password'} />
    </label>
    {#if creating}
      <label>
        Repeat password
        <input type="password" bind:value={confirm} required autocomplete="new-password" />
      </label>
      <p class="help">
        {#if desktop}
          At least 8 characters. It encrypts the passwords and keys you save on this computer. If
          you forget it, they cannot be recovered.
        {:else}
          At least 8 characters. This password also encrypts the passwords and keys you save. If you
          forget it, they cannot be recovered.
        {/if}
      </p>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <button type="submit" class="primary" disabled={busy}>
      {desktop ? (creating ? 'Continue' : 'Unlock') : creating ? 'Create account' : 'Sign in'}
    </button>
    {#if session.google}
      <button type="button" disabled={busy} onclick={() => (location.href = '/api/auth/google')}>Continue with Google</button>
    {/if}
    {#if !setup && session.signup}
      <button type="button" class="plain switch" onclick={toggle}>
        {creating ? 'I already have an account' : 'Create an account'}
      </button>
    {/if}
  </form>
</div>
