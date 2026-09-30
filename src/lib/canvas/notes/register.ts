// Registers the node kinds of the notes/files area: `note` (full .md note,
// overrides the engine placeholder), `stack` (fichário), `filetree` and
// `file` (file card, or editor with data.view = "editor"). Imported once by
// Canvas.svelte; registration without `ifAbsent` wins over the built-ins in
// any import order.

import { ICONS, registerNodeKind } from '../nodes/registry';
import NoteNodeFull from '$components/canvas/nodes/notes/NoteNodeFull.svelte';
import StackNode from '$components/canvas/nodes/notes/StackNode.svelte';
import FileTreeNode from '$components/canvas/nodes/files/FileTreeNode.svelte';
import FileEditorNode from '$components/canvas/nodes/files/FileEditorNode.svelte';

const STACK_ICON = 'M6 7h12v13H6z M8 4h10v2 M10 1.5h8';
const TREE_ICON = 'M3 6h6l2 2h10v11H3z';

let done = false;

export function registerNotesAndFiles(): void {
  if (done) return;
  done = true;
  registerNodeKind('note', {
    component: NoteNodeFull,
    minSize: { w: 160, h: 100 },
    defaultSize: { w: 260, h: 200 },
    icon: ICONS.note,
    labelKey: 'canvas.notes.kind',
  });
  registerNodeKind('stack', {
    component: StackNode,
    minSize: { w: 200, h: 140 },
    defaultSize: { w: 280, h: 240 },
    icon: STACK_ICON,
    labelKey: 'canvas.notes.stackKind',
  });
  registerNodeKind('filetree', {
    component: FileTreeNode,
    minSize: { w: 200, h: 160 },
    defaultSize: { w: 320, h: 400 },
    icon: TREE_ICON,
    labelKey: 'canvas.files.treeKind',
  });
  registerNodeKind('file', {
    component: FileEditorNode,
    minSize: { w: 160, h: 60 },
    defaultSize: { w: 200, h: 160 },
    icon: ICONS.file,
    labelKey: 'canvas.files.fileKind',
  });
}

registerNotesAndFiles();
