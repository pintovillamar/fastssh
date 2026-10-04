<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { Terminal } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import { WebglAddon } from '@xterm/addon-webgl'
  import KeyBar, { type BarKey, type Modifiers } from './KeyBar.svelte'
  import { applyModifiers, barKeySequence } from './keys'

  let { connectionId, visible }: { connectionId: number | null; visible: boolean } = $props()

  type Question =
    | { type: 'host_key'; host: string; port: number; algorithm: string; fingerprint: string }
    | { type: 'prompt'; kind: 'password' | 'passphrase'; message: string }

  let host: HTMLDivElement
  let question = $state<Question | null>(null)
  let secret = $state('')
  let secretInput = $state<HTMLInputElement>()

  let term: Terminal | undefined
  let fit: FitAddon | undefined
  let socket: WebSocket | undefined

  // The extra-keys bar is for devices whose main input is touch.
  const touch = matchMedia('(pointer: coarse)')
  let showKeyBar = $state(touch.matches)
  let modifiers = $state<Modifiers>({ ctrl: false, alt: false })
  const encoder = new TextEncoder()

  function sendInput(text: string) {
    if (socket?.readyState === WebSocket.OPEN) socket.send(encoder.encode(text))
  }

  function pressBarKey(key: BarKey) {
    sendInput(barKeySequence(key, modifiers, term?.modes.applicationCursorKeysMode ?? false))
    modifiers = { ctrl: false, alt: false }
    term?.focus()
  }

  /** Text from the keyboard or the bar's symbol keys, with armed modifiers applied. */
  function typeText(text: string) {
    if (modifiers.ctrl || modifiers.alt) {
      const modified = applyModifiers(text, modifiers)
      if (modified !== null) {
        modifiers = { ctrl: false, alt: false }
        text = modified
      }
    }
    sendInput(text)
  }

  function sendControl(message: object) {
    if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message))
  }

  function answerHostKey(accept: boolean) {
    sendControl({ type: 'host_key_reply', accept })
    question = null
    term?.focus()
  }

  function answerSecret(event: SubmitEvent) {
    event.preventDefault()
    sendControl({ type: 'secret', value: secret })
    secret = ''
    question = null
    term?.focus()
  }

  function cancelSecret() {
    sendControl({ type: 'cancel' })
    secret = ''
    question = null
    term?.focus()
  }

  // A hidden terminal has no size, so it is fitted again when its tab returns.
  $effect(() => {
    if (visible && term) {
      fit?.fit()
      if (!question) term.focus()
    }
  })

  onMount(() => {
    const css = getComputedStyle(document.documentElement)
    const color = (name: string) => css.getPropertyValue(name).trim()
    const t = new Terminal({
      cursorBlink: true,
      fontFamily: 'ui-monospace, "JetBrains Mono", "Fira Code", Menlo, monospace',
      fontSize: 14,
      scrollback: 10000,
      theme: {
        background: color('--window'),
        foreground: color('--text'),
        cursor: color('--accent'),
        selectionBackground: color('--accent') + '55',
      },
    })
    const f = new FitAddon()
    t.loadAddon(f)
    t.open(host)
    try {
      // GPU rendering is much faster, but not every device offers it.
      const webgl = new WebglAddon()
      webgl.onContextLoss(() => webgl.dispose())
      t.loadAddon(webgl)
    } catch {
      // The default DOM renderer keeps working.
    }
    f.fit()
    term = t
    fit = f

    const dim = (text: string) => t.write(`\r\n\x1b[2m${text}\x1b[0m\r\n`)

    async function onControl(message: any) {
      switch (message.type) {
        case 'host_key':
        case 'prompt':
          question = message
          if (message.type === 'prompt') {
            await tick()
            secretInput?.focus()
          }
          break
        case 'error':
          if (message.message !== 'cancelled') t.write(`\r\n\x1b[31m${message.message}\x1b[0m\r\n`)
          break
      }
    }

    function connect() {
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
      const target = connectionId === null ? '' : `&connection=${connectionId}`
      const ws = new WebSocket(
        `${scheme}://${location.host}/ws?cols=${t.cols}&rows=${t.rows}${target}`,
      )
      ws.binaryType = 'arraybuffer'
      ws.onmessage = (event) => {
        if (event.data instanceof ArrayBuffer) t.write(new Uint8Array(event.data))
        else onControl(JSON.parse(event.data))
      }
      ws.onclose = () => {
        if (socket !== ws) return
        socket = undefined
        question = null
        dim('[session ended — press Enter to start a new one]')
      }
      socket = ws
      if (connectionId !== null) t.write('\x1b[2mConnecting…\x1b[0m\r\n')
    }

    function send(data: string | Uint8Array<ArrayBuffer>) {
      if (socket?.readyState === WebSocket.OPEN) socket.send(data)
    }

    t.onData((data) => {
      if (socket) typeText(data)
      else if (data === '\r') connect()
    })
    // Mouse reports can contain bytes that are not valid UTF-8.
    t.onBinary((data) => send(Uint8Array.from(data, (c) => c.charCodeAt(0))))
    t.onResize(({ cols, rows }) => send(JSON.stringify({ type: 'resize', cols, rows })))

    const onTouchChange = () => (showKeyBar = touch.matches)
    touch.addEventListener('change', onTouchChange)

    const observer = new ResizeObserver(() => {
      // Fitting a hidden terminal would shrink the remote shell to nothing.
      if (host.clientWidth > 0 && host.clientHeight > 0) f.fit()
    })
    observer.observe(host)

    connect()
    t.focus()

    return () => {
      touch.removeEventListener('change', onTouchChange)
      observer.disconnect()
      const ws = socket
      socket = undefined
      ws?.close()
      t.dispose()
      term = undefined
    }
  })
</script>

<div class="session">
  <div class="terminal" bind:this={host}></div>
  {#if showKeyBar}
    <KeyBar bind:modifiers onkey={pressBarKey} ontext={(text) => { typeText(text); term?.focus() }} />
  {/if}
</div>

{#if question}
  <div class="backdrop">
    {#if question.type === 'host_key'}
      <div class="dialog" role="alertdialog" aria-labelledby="q-title">
        <h2 id="q-title">Trust this server?</h2>
        <p>
          This is the first connection to <strong>{question.host}:{question.port}</strong>. Check that
          its key fingerprint matches what the server's owner gave you.
        </p>
        <code>{question.algorithm} {question.fingerprint}</code>
        <div class="actions">
          <button class="primary" onclick={() => answerHostKey(true)}>Trust and connect</button>
          <button onclick={() => answerHostKey(false)}>Cancel</button>
        </div>
      </div>
    {:else}
      <form class="dialog" onsubmit={answerSecret}>
        <label>
          {question.message}
          <input
            type="password"
            bind:value={secret}
            bind:this={secretInput}
            autocomplete={question.kind === 'password' ? 'current-password' : 'off'}
          />
        </label>
        <div class="actions">
          <button type="submit" class="primary">Continue</button>
          <button type="button" onclick={cancelSecret}>Cancel</button>
        </div>
      </form>
    {/if}
  </div>
{/if}

<style>
  .session {
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  .terminal {
    flex: 1;
    min-height: 0;
    padding: 6px;
    box-sizing: border-box;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 16px;
    background: #000000b0;
    z-index: 10;
  }

  .dialog {
    width: 100%;
    max-width: 440px;
    box-sizing: border-box;
    display: grid;
    gap: 14px;
    padding: 20px;
    background: var(--surface);
    border: 1px solid var(--outline);
    border-radius: 14px;
  }

  h2 {
    margin: 0;
    font-size: 17px;
  }

  p {
    margin: 0;
    color: var(--secondary);
    line-height: 1.45;
  }

  code {
    font-size: 13px;
    word-break: break-all;
    padding: 10px 12px;
    background: var(--window);
    border: 1px solid var(--outline);
    border-radius: var(--radius);
  }

  label {
    font-size: 14px;
    color: var(--text);
    word-break: break-word;
  }

  .actions {
    display: flex;
    gap: 8px;
  }
</style>
