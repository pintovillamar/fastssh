import { describe, expect, it } from 'vitest'
import { applyModifiers, barKeySequence } from './keys'

const none = { ctrl: false, alt: false }
const ctrl = { ctrl: true, alt: false }
const alt = { ctrl: false, alt: true }

describe('barKeySequence', () => {
  it('sends plain arrows in the mode the program asked for', () => {
    expect(barKeySequence('up', none, false)).toBe('\x1b[A')
    expect(barKeySequence('up', none, true)).toBe('\x1bOA')
    expect(barKeySequence('left', none, false)).toBe('\x1b[D')
    expect(barKeySequence('end', none, false)).toBe('\x1b[F')
  })

  it('encodes modifiers the xterm way', () => {
    expect(barKeySequence('right', ctrl, false)).toBe('\x1b[1;5C')
    expect(barKeySequence('right', alt, true)).toBe('\x1b[1;3C')
    expect(barKeySequence('left', { ctrl: true, alt: true }, false)).toBe('\x1b[1;7D')
    expect(barKeySequence('pageup', none, false)).toBe('\x1b[5~')
    expect(barKeySequence('pagedown', ctrl, false)).toBe('\x1b[6;5~')
  })

  it('sends Esc and Tab', () => {
    expect(barKeySequence('esc', none, false)).toBe('\x1b')
    expect(barKeySequence('tab', none, false)).toBe('\t')
    expect(barKeySequence('tab', alt, false)).toBe('\x1b\t')
  })
})

describe('applyModifiers', () => {
  it('turns Ctrl+letter into a control character', () => {
    expect(applyModifiers('c', ctrl)).toBe('\x03')
    expect(applyModifiers('C', ctrl)).toBe('\x03')
    expect(applyModifiers('d', ctrl)).toBe('\x04')
    expect(applyModifiers('[', ctrl)).toBe('\x1b')
    expect(applyModifiers(' ', ctrl)).toBe('\x00')
  })

  it('prefixes Alt with escape', () => {
    expect(applyModifiers('b', alt)).toBe('\x1bb')
    expect(applyModifiers('c', { ctrl: true, alt: true })).toBe('\x1b\x03')
  })

  it('leaves characters Ctrl has no meaning for', () => {
    expect(applyModifiers('1', ctrl)).toBe('1')
  })

  it('does not touch pasted or autocompleted text', () => {
    expect(applyModifiers('ls -la', ctrl)).toBeNull()
    expect(applyModifiers('', ctrl)).toBeNull()
  })
})
