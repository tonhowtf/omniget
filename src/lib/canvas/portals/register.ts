import PortalNode from '$components/canvas/nodes/PortalNode.svelte';
import { DEFAULT_SIZES, ICONS, MIN_SIZES, registerNodeKind } from '../nodes/registry';

let done = false;

function registerPortalNodes(): void {
  if (done) return;
  done = true;
  registerNodeKind('portal', {
    component: PortalNode,
    minSize: MIN_SIZES.portal,
    defaultSize: DEFAULT_SIZES.portal,
    icon: ICONS.portal,
    focusOnSecondClick: true,
    labelKey: 'canvas.portal.kind',
  });
}

registerPortalNodes();
