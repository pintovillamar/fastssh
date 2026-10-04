import { mount } from 'svelte'
import '@xterm/xterm/css/xterm.css'
import './app.css'
import App from './App.svelte'

mount(App, { target: document.getElementById('app')! })
