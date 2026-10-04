// Turning key presses into the bytes a terminal expects.

import type { BarKey, Modifiers } from './KeyBar.svelte'

const ESC = '\x1b'

/**
 * The escape sequence for a key on the extra-keys bar.
 *
 * `applicationCursor` is a mode programs like vim switch on, in which plain
 * arrows are sent as `ESC O A` instead of `ESC [ A`.
 */
export function barKeySequence(key: BarKey, modifiers: Modifiers, applicationCursor: boolean): string {
  if (key === 'esc') return modifiers.alt ? ESC + ESC : ESC
  if (key === 'tab') return modifiers.alt ? ESC + '\t' : '\t'

  // xterm's modifier parameter: 1 + 2 for Alt + 4 for Ctrl.
  const modifier = 1 + (modifiers.alt ? 2 : 0) + (modifiers.ctrl ? 4 : 0)
  if (key === 'pageup' || key === 'pagedown') {
    const code = key === 'pageup' ? 5 : 6
    return modifier === 1 ? `${ESC}[${code}~` : `${ESC}[${code};${modifier}~`
  }
  const letter = { up: 'A', down: 'B', right: 'C', left: 'D', home: 'H', end: 'F' }[key]
  if (modifier !== 1) return `${ESC}[1;${modifier}${letter}`
  return `${ESC}${applicationCursor ? 'O' : '['}${letter}`
}

/**
 * Applies armed modifiers to text typed on the keyboard. Only a single
 * character can take a modifier; anything longer (a paste, an autocompleted
 * word) is returned as `null` and should be sent unchanged.
 */
export function applyModifiers(text: string, modifiers: Modifiers): string | null {
  if ([...text].length !== 1) return null
  let out = text
  if (modifiers.ctrl) {
    const code = text.toUpperCase().charCodeAt(0)
    // Ctrl maps @ A-Z [ \ ] ^ _ onto the control characters 0-31.
    if (code >= 64 && code <= 95) out = String.fromCharCode(code & 0x1f)
    else if (text === ' ') out = '\x00'
    else if (text === '?') out = '\x7f'
  }
  return modifiers.alt ? ESC + out : out
}
