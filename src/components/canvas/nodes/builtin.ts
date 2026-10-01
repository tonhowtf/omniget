import type { Component } from 'svelte';
import { ICONS, MIN_SIZES, DEFAULT_SIZES, registerNodeKind } from '$lib/canvas/nodes/registry';

const modules = import.meta.glob<{ default: Component<any> }>('./*.svelte', { eager: true }); // eslint-disable-line @typescript-eslint/no-explicit-any

const BY_FILE: Record<string, { kinds: string[]; icon: string; focusOnClick?: boolean; focusOnSecondClick?: boolean }> = {
  'NoteNode.svelte': { kinds: ['note'], icon: ICONS.note },
  'TextNode.svelte': { kinds: ['text'], icon: ICONS.text },
  'FileNode.svelte': { kinds: ['file'], icon: ICONS.file },
  'LinkNode.svelte': { kinds: ['link'], icon: ICONS.link },
  'ImageNode.svelte': { kinds: ['image'], icon: ICONS.image },
  'GenerationNode.svelte': { kinds: ['generation'], icon: ICONS.generation, focusOnClick: true },
  'UnknownNode.svelte': { kinds: ['unknown'], icon: ICONS.unknown },
  'ConnectorNode.svelte': { kinds: ['connector'], icon: ICONS.link },
  'TerminalNode.svelte': { kinds: ['terminal', 'agent'], icon: ICONS.terminal, focusOnClick: true },
  'PortalNode.svelte': { kinds: ['portal'], icon: ICONS.portal, focusOnSecondClick: true },
  'FileTreeNode.svelte': { kinds: ['filetree'], icon: ICONS.file },
  'NoteStackNode.svelte': { kinds: ['stack'], icon: ICONS.note },
};

let done = false;

export function registerBuiltinNodes(): void {
  if (done) return;
  done = true;
  for (const [path, mod] of Object.entries(modules)) {
    const file = path.split('/').pop() ?? '';
    const spec = BY_FILE[file];
    if (!spec || !mod?.default) continue;
    for (const kind of spec.kinds) {
      registerNodeKind(
        kind,
        {
          component: mod.default,
          minSize: MIN_SIZES[kind] ?? { w: 40, h: 24 },
          defaultSize: DEFAULT_SIZES[kind] ?? DEFAULT_SIZES.unknown,
          icon: spec.icon,
          focusOnClick: spec.focusOnClick,
          focusOnSecondClick: spec.focusOnSecondClick,
        },
        { ifAbsent: true },
      );
    }
  }
}
