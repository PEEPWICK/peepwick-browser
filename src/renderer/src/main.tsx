import '@fontsource-variable/archivo/standard.css'
import '@fontsource/ibm-plex-mono/400.css'
import '@fontsource/ibm-plex-mono/500.css'
import '@fontsource-variable/newsreader/standard.css'
import './styles/tokens.css'
import './styles/chrome.css'
import './styles/panels.css'
import './styles/pages.css'
import { createRoot } from 'react-dom/client'
import { App } from './App'
import { initStore } from './store'

void initStore().then(() => {
  createRoot(document.getElementById('root')!).render(<App />)
})
