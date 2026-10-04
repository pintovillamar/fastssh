<script lang="ts">
  import { onMount } from 'svelte'
  import { Terminal } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import { WebglAddon } from '@xterm/addon-webgl'

  let host: HTMLDivElement

  onMount(() => {
    const css = getComputedStyle(document.documentElement)
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: 'ui-monospace, "JetBrains Mono", "Fira Code", Menlo, monospace',
      fontSize: 14,
      scrollback: 10000,
      theme: {
        background: css.getPropertyValue('--window').trim(),
        foreground: css.getPropertyValue('--text').trim(),
        cursor: css.getPropertyValue('--accent').trim(),
        selectionBackground: css.getPropertyValue('--accent').trim() + '55',
      },
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(host)
    try {
      // GPU rendering is much faster, but not every device offers it.
      const webgl = new WebglAddon()
      webgl.onContextLoss(() => webgl.dispose())
      term.loadAddon(webgl)
    } catch {
      // The default DOM renderer keeps working.
    }
    fit.fit()

    const encoder = new TextEncoder()
    let socket: WebSocket | undefined

    function connect() {
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
      const ws = new WebSocket(
        `${scheme}://${location.host}/ws?cols=${term.cols}&rows=${term.rows}`,
      )
      ws.binaryType = 'arraybuffer'
      ws.onmessage = (event) => {
        if (event.data instanceof ArrayBuffer) term.write(new Uint8Array(event.data))
      }
      ws.onclose = () => {
        if (socket !== ws) return
        socket = undefined
        term.write('\r\n\x1b[2m[session ended — press Enter to start a new one]\x1b[0m\r\n')
      }
      socket = ws
    }

    function send(data: string | Uint8Array<ArrayBuffer>) {
      if (socket?.readyState === WebSocket.OPEN) socket.send(data)
    }

    term.onData((data) => {
      if (socket) send(encoder.encode(data))
      else if (data === '\r') connect()
    })
    // Mouse reports can contain bytes that are not valid UTF-8.
    term.onBinary((data) => send(Uint8Array.from(data, (c) => c.charCodeAt(0))))
    term.onResize(({ cols, rows }) => send(JSON.stringify({ type: 'resize', cols, rows })))

    const observer = new ResizeObserver(() => fit.fit())
    observer.observe(host)

    connect()
    term.focus()

    return () => {
      observer.disconnect()
      const ws = socket
      socket = undefined
      ws?.close()
      term.dispose()
    }
  })
</script>

<div class="terminal" bind:this={host}></div>

<style>
  .terminal {
    height: 100%;
    padding: 6px;
    box-sizing: border-box;
  }
</style>
