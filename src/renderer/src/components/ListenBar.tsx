// Read aloud: speaks the article of reader view or of a saved reading-list page with
// the system's voices (speech-dispatcher on Linux). Paragraph by paragraph, so it
// can skip back and forth; pausing stops at the paragraph and resumes from its start,
// which works the same with every speech engine.
import { useEffect, useMemo, useRef, useState } from 'react'
import { Headphones, Pause, Play, SkipBack, SkipForward, X } from 'lucide-react'
import { speechChunks } from '@shared/readaloud'
import type { ReadAloudText, TabState } from '@shared/types'
import { call } from '../api'
import { useT } from '../store'
import { Icon, IconButton } from '../ui'

const RATES = [0.75, 1, 1.25, 1.5, 1.75, 2]

/** Whether read aloud has something to read in this tab. */
export function canListen(tab: TabState | null | undefined): boolean {
  return !!tab && (tab.reader || tab.url.startsWith('peepwick://saved/'))
}

function useVoices(): SpeechSynthesisVoice[] {
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>(() => speechSynthesis.getVoices())
  useEffect(() => {
    const update = () => setVoices(speechSynthesis.getVoices())
    speechSynthesis.addEventListener('voiceschanged', update)
    update()
    return () => speechSynthesis.removeEventListener('voiceschanged', update)
  }, [])
  return voices
}

/** Voices for a language; engines like espeak-ng list dozens of "+variant" copies of each. */
function voicesFor(all: SpeechSynthesisVoice[], lang: string): SpeechSynthesisVoice[] {
  const base = lang.toLowerCase().split('-')[0]
  const matching = all.filter((v) => v.lang.toLowerCase().split('-')[0] === base)
  const plain = matching.filter((v) => !v.name.includes('+'))
  return (plain.length > 0 ? plain : matching).slice(0, 40)
}

export function ListenBar({ tab, uiLang, onClose }: { tab: TabState; uiLang: string; onClose: () => void }) {
  const t = useT()
  const all = useVoices()
  const [text, setText] = useState<ReadAloudText | null>(null)
  const [index, setIndex] = useState(0)
  const [playing, setPlaying] = useState(false)
  const [rate, setRate] = useState(1)
  const [voiceName, setVoiceName] = useState<string | null>(null)
  /** Bumped on every start and stop, so a cancelled utterance's events are ignored. */
  const run = useRef(0)

  const reader = tab.reader
  useEffect(() => {
    void call.readAloud(tab.id).then(setText)
  }, [tab.id, tab.url, reader])

  const chunks = useMemo(() => (text ? [text.title, ...text.paragraphs].filter(Boolean).flatMap((p) => speechChunks(p)) : []), [text])
  const lang = text?.lang || uiLang
  const voices = useMemo(() => voicesFor(all, lang), [all, lang])
  const voice = voices.find((v) => v.name === voiceName) ?? voices.find((v) => v.default) ?? voices[0] ?? null

  const stop = () => {
    run.current++
    speechSynthesis.cancel()
    setPlaying(false)
  }

  const speak = (at: number) => {
    const id = ++run.current
    speechSynthesis.cancel()
    if (at >= chunks.length) {
      setPlaying(false)
      setIndex(0)
      return
    }
    const utterance = new SpeechSynthesisUtterance(chunks[at])
    utterance.lang = voice?.lang ?? lang
    if (voice) utterance.voice = voice
    utterance.rate = rate
    utterance.onend = () => id === run.current && speak(at + 1)
    utterance.onerror = (event) => {
      if (id === run.current && event.error !== 'interrupted' && event.error !== 'canceled') setPlaying(false)
    }
    setIndex(at)
    setPlaying(true)
    speechSynthesis.speak(utterance)
  }

  // Start reading as soon as the text arrives; stop when the bar goes away.
  const started = useRef(false)
  useEffect(() => {
    if (chunks.length > 0 && !started.current) {
      started.current = true
      speak(0)
    }
  }, [chunks.length])
  useEffect(() => () => stop(), [])

  // A new speed or voice applies from the current paragraph.
  const restart = useRef(false)
  useEffect(() => {
    if (!restart.current) {
      restart.current = true
      return
    }
    if (playing) speak(index)
  }, [rate, voice?.name])

  // The text is gone when reader view closes or the tab leaves the saved page.
  useEffect(() => {
    if (!canListen(tab)) onClose()
  }, [tab.reader, tab.url])

  const noVoices = all.length === 0
  return (
    <div className="bar listen-bar" role="region" aria-label={t('listen.title')}>
      <Icon icon={Headphones} size={16} className="muted" />
      <span className="listen-label">{t('listen.title')}</span>
      {noVoices ? (
        <span className="muted listen-note">{t('listen.noVoices')}</span>
      ) : (
        <>
          <IconButton icon={SkipBack} label={t('listen.prev')} disabled={!text || index === 0} onClick={() => speak(Math.max(0, index - 1))} />
          <IconButton icon={playing ? Pause : Play} label={playing ? t('listen.pause') : t('listen.play')} disabled={!text} onClick={() => (playing ? stop() : speak(index))} />
          <IconButton icon={SkipForward} label={t('listen.next')} disabled={!text || index >= chunks.length - 1} onClick={() => speak(index + 1)} />
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
              <select className="select" value={voice?.name ?? ''} onChange={(event) => setVoiceName(event.target.value)}>
                {voices.map((v) => (
                  <option key={v.name} value={v.name}>
                    {v.name.replace(/ espeak-ng$/, '')}
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
