import { useEffect, useState } from 'react'
import { ExternalLink, Puzzle } from 'lucide-react'
import type { ExtensionInfo } from '@shared/types'
import { api, call } from '../api'
import { useSnapshot, useT } from '../store'
import { Button, Icon } from '../ui'

export function Extensions() {
  const { state } = useSnapshot()
  const t = useT()
  const [list, setList] = useState<ExtensionInfo[] | null>(null)

  useEffect(() => {
    const load = () => void call.extensionsList().then(setList)
    load()
    const id = setInterval(load, 2000)
    return () => clearInterval(id)
  }, [])

  return (
    <div className="page list-page">
      <div className="list-page-inner">
        <header className="list-head">
          <h1 className="display small">{t('ext.title')}</h1>
        </header>
        {state.incognito ? (
          <p className="lead">{t('ext.incognito')}</p>
        ) : (
          <>
            <p className="lead">{t('ext.lead')}</p>
            <div className="ext-actions">
              <Button variant="primary" onClick={() => api.openURL('https://chromewebstore.google.com/', true)}>
                {t('ext.store')} <Icon icon={ExternalLink} size={14} />
              </Button>
            </div>
            {list && list.length === 0 && <p className="muted list-empty">{t('ext.empty')}</p>}
            <ul className="ext-list">
              {list?.map((ext) => (
                <li key={ext.id} className="ext-card">
                  <span className="ext-icon">
                    {ext.icon ? <img src={ext.icon} width={32} height={32} alt="" /> : <Icon icon={Puzzle} size={24} />}
                  </span>
                  <div className="ext-text">
                    <div className="ext-name">
                      {ext.name} <span className="mono muted">{ext.version}</span>
                    </div>
                    {ext.description && <p className="ext-desc">{ext.description}</p>}
                    <span className="mono muted ext-id">{ext.id}</span>
                  </div>
                  <div className="ext-buttons">
                    {ext.optionsUrl && (
                      <Button variant="quiet" onClick={() => api.extensionOptions(ext.id)}>
                        {t('ext.options')}
                      </Button>
                    )}
                    <Button onClick={() => api.extensionRemove(ext.id)}>{t('ext.remove')}</Button>
                  </div>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </div>
  )
}
