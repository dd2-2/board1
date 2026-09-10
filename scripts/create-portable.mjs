import fs from 'node:fs/promises'
import path from 'node:path'

const cwd = process.cwd()
const exeSrc = path.join(cwd, 'src-tauri', 'target', 'release', 'board1.exe')
const out = path.join(cwd, '..', 'build')

await fs.mkdir(out, { recursive: true })
await fs.copyFile(exeSrc, path.join(out, 'board1.exe'))

console.log(`\n포터블 빌드 완료: ${path.join(out, 'board1.exe')}`)
