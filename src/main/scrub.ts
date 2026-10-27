// Takes out of an error message anything that could identify the person or what
// they were doing: addresses, their home folder and user name, e-mail and IP
// addresses, and long tokens. Used before an error enters the bug report log.
import { homedir, userInfo } from 'node:os'

let userName = ''
try {
  userName = userInfo().username
} catch {
  userName = ''
}

/** Removes anything that could identify the person or what they were doing. */
export function scrub(text: string, home = homedir(), user = userName): string {
  let out = text
  if (home && home.length > 1) out = out.split(home).join('~')
  out = out
    .replace(/\b[a-z][a-z0-9+.-]*:\/\/[^\s'"<>()[\]{}]+/gi, '<url>')
    // File paths: the browser's own code keeps its place (for line numbers), any
    // other path, with its folder and file names, becomes <path>.
    .replace(/(^|[\s('"=:])(~\/[^\s'"()<>,;]+|\/[^\s'"()<>,;/][^\s'"()<>,;]*)/g, (_match, lead: string, path: string) => {
      const own = /\/(out\/(?:main|preload|renderer)\/[^/]+)$/.exec(path)
      return `${lead}${own ? own[1] : '<path>'}`
    })
    .replace(/\b(?:data|blob|javascript|mailto|about):[^\s'"<>()]+/gi, '<url>')
    .replace(/[\w.+-]+@[\w-]+(\.[\w-]+)+/g, '<email>')
    .replace(/\b(?:\d{1,3}\.){3}\d{1,3}\b/g, '<ip>')
    .replace(/\b[A-Za-z0-9_-]{32,}\b/g, '<token>')
  if (user && user.length > 2) out = out.replace(new RegExp(`\\b${user.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`, 'g'), '<user>')
  return out.slice(0, 600)
}
