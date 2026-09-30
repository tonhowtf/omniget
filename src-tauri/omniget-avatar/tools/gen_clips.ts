// Bakes the procedural clips of src/lib/city3d/avatar/clips.ts into
// static/city3d/clips/clips.json. Run from the repo root:
//   pnpm exec esbuild src-tauri/omniget-avatar/tools/gen_clips.ts --bundle \
//     --platform=node --format=esm --outfile=<tmp>/gen_clips.mjs && node <tmp>/gen_clips.mjs
import { writeFileSync } from 'node:fs';
import { generateClips } from '../../../src/lib/city3d/avatar/clips';

const clips = generateClips();
const out = process.argv[2] ?? 'static/city3d/clips/clips.json';
const body = JSON.stringify({ schema: 'omniget.clips/1', rig: 'vrm-1.0-tpose', clips });
writeFileSync(out, body);
console.log(`${clips.length} clips, ${body.length} bytes -> ${out}`);
