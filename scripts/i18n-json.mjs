// Writes the interface translations (src/shared/i18n.ts) as JSON, so the Rust side
// uses the same strings. Run by src-tauri/build.rs; Node strips the TypeScript types.
import { writeFileSync } from 'node:fs'
import { dictionariesForTest } from '../src/shared/i18n.ts'

writeFileSync(process.argv[2], JSON.stringify(dictionariesForTest))
