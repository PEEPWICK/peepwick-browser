// Read aloud: speaks the article of reader view or of a saved reading-list page with
// the system's voices: speech-dispatcher, which the Rust side talks to. Paragraph by
// paragraph, so it can skip back and forth; pausing stops at the paragraph and resumes
// from its start, which works the same with every speech engine.
import { useEffect, useMemo, useRef, useState } from 'react'
import { Headphones, Pause, Play, SkipBack, SkipForward, X } from 'lucide-react'
import { speechChunks } from '@shared/readaloud'
import type { ReadAloudText, SpeechVoice, TabState } from '@shared/types'
import { api, call, on } from '../api'
import { useT } from '../store'
import { Icon, IconButton } from '../ui'

const RATES = [0.75, 1, 1.25, 1.5, 1.75, 2]

/** Whether read aloud has something to read in this tab. Without speech-dispatcher the
 * bar says what to install. */
export function canListen(tab: TabState | null | undefined): boolean {
  return !!tab && (tab.reader || tab.url.startsWith('peepwick://saved/'))
}

/** speech-dispatcher's voices; null until they come. */
function useVoices(): SpeechVoice[] | null {
  const [voices, setVoices] = useState<SpeechVoice[] | null>(null)
  useEffect(() => {
    let live = true
    const done = (list: SpeechVoice[]) => live && setVoices(list)
    void call.speechVoices().then((list) => done(list ?? []), () => done([]))
    return () => {
      live = false
    }
  }, [])
  return voices
}

/** Voices for a language, the closest first: "en-US" before the other Englishes. */
function voicesFor(all: SpeechVoice[], lang: string): SpeechVoice[] {
  const tag = lang.toLowerCase()
  const base = tag.split('-')[0]
  const matching = all.filter((v) => v.lang.toLowerCase().split('-')[0] === base)
  const exact = matching.filter((v) => v.lang.toLowerCase() === tag)
  return [...exact, ...matching.filter((v) => !exact.includes(v))].slice(0, 40)
}

const voiceId = (v: SpeechVoice) => `${v.module}/${v.name}`

/** Ids for the paragraphs this window sends; speech events carry them back. */
let lastKey = 0

export function ListenBar({ tab, uiLang, onClose }: { tab: TabState; uiLang: string; onClose: () => void }) {
  const t = useT()
  const all = useVoices()
  const [text, setText] = useState<ReadAloudText | null>(null)
  const [index, setIndex] = useState(0)
  const [playing, setPlaying] = useState(false)
  const [rate, setRate] = useState(1)
  /** The voice the reader picked; without one speech-dispatcher picks for the language. */
  const [picked, setPicked] = useState<string | null>(null)
  /** The paragraph being said; a stopped or replaced one's events are ignored. */
  const run = useRef<{ key: string; at: number } | null>(null)

  const reader = tab.reader
  useEffect(() => {
    void call.readAloud(tab.id).then(setText)
  }, [tab.id, tab.url, reader])

  const chunks = useMemo(() => (text ? [text.title, ...text.paragraphs].filter(Boolean).flatMap((p) => speechChunks(p)) : []), [text])
  const lang = text?.lang || uiLang
  const voices = useMemo(() => voicesFor(all ?? [], lang), [all, lang])
  const voice = (picked && voices.find((v) => voiceId(v) === picked)) || null
  const engines = new Set(voices.map((v) => v.module)).size

  const stop = () => {
    run.current = null
    api.speechStop()
    setPlaying(false)
  }

  const speak = (at: number) => {
    if (at >= chunks.length) {
      run.current = null
      setPlaying(false)
      setIndex(0)
      return
    }
    const key = String(++lastKey)
    run.current = { key, at }
    setIndex(at)
    setPlaying(true)
    api.speak({ key, text: chunks[at], lang, voice: voice && { module: voice.module, name: voice.name }, rate })
  }

  // A paragraph's end starts the next one. A stop from elsewhere (another window began
  // reading, speech-dispatcher went away) pauses.
  const latest = useRef(speak)
  latest.current = speak
  useEffect(
    () =>
      on('speech', ({ key, event }) => {
        const current = run.current
        if (!current || current.key !== key) return
        if (event === 'end') latest.current(current.at + 1)
        else if (event === 'stopped' || event === 'error') {
          run.current = null
          setPlaying(false)
        }
      }),
    [],
  )

  // Start reading once the text and the voices are there; stop when the bar goes away.
  const started = useRef(false)
  useEffect(() => {
    if (chunks.length > 0 && all && all.length > 0 && !started.current) {
      started.current = true
      speak(0)
    }
  }, [chunks.length, all])
  useEffect(() => () => stop(), [])

  // A new speed or voice applies from the current paragraph.
  const restart = useRef(false)
  useEffect(() => {
    if (!restart.current) {
      restart.current = true
      return
    }
    if (playing) speak(index)
  }, [rate, picked])

  const noVoices = all !== null && all.length === 0
  const ready = !!text && !!all
  return (
    <div className="bar listen-bar" role="region" aria-label={t('listen.title')}>
      <Icon icon={Headphones} size={16} className="muted" />
      <span className="listen-label">{t('listen.title')}</span>
      {noVoices ? (
        <span className="muted listen-note">{t('listen.noVoices')}</span>
      ) : (
        <>
          <IconButton icon={SkipBack} label={t('listen.prev')} disabled={!ready || index === 0} onClick={() => speak(Math.max(0, index - 1))} />
          <IconButton icon={playing ? Pause : Play} label={playing ? t('listen.pause') : t('listen.play')} disabled={!ready} onClick={() => (playing ? stop() : speak(index))} />
          <IconButton icon={SkipForward} label={t('listen.next')} disabled={!ready || index >= chunks.length - 1} onClick={() => speak(index + 1)} />
          <span className="mono muted listen-progress">{chunks.length > 0 ? t('listen.progress', { n: index + 1, total: chunks.length }) : ''}</span>
          <span className="spacer" />
          <label className="listen-field">
            <span className="label">{t('listen.speed')}</span>
            <select className="select" value={rate} onChange={(event) => setRate(Number(event.target.value))}>
              {RATES.map((r) => (
                <option key={r} value={r}>
                  {r}×
                </option>
              ))}
            </select>
          </label>
          {voices.length > 1 && (
            <label className="listen-field">
              <span className="label">{t('listen.voice')}</span>
              <select className="select" value={voice ? voiceId(voice) : ''} onChange={(event) => setPicked(event.target.value || null)}>
                <option value="">{t('listen.voiceDefault')}</option>
                {voices.map((v) => (
                  <option key={voiceId(v)} value={voiceId(v)}>
                    {engines > 1 ? `${v.name} (${v.module})` : v.name}
                  </option>
                ))}
              </select>
            </label>
          )}
        </>
      )}
      <IconButton
        icon={X}
        label={t('common.close')}
        onClick={() => {
          stop()
          onClose()
        }}
      />
    </div>
  )
}
