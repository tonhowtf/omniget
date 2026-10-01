/**
 * Bot avatars (libraries.dev `bot-avatars`) as a React island inside Svelte.
 *
 * `bot-avatars` ships only a React component, so this action loads React,
 * React DOM and the package with a dynamic import (they land in their own
 * chunk, fetched the first time a card opens in the limits-strip window and
 * never by any other screen) and renders one `<BotAvatar>` into the node.
 *
 * Only documented props are passed: `type`, `state` ("default" | "working"),
 * `size`, `paused`, plus pass-through canvas attributes. The package runs one
 * shared frame loop, leaves it when the avatar is offscreen or the tab is
 * hidden, and draws a still pose under prefers-reduced-motion.
 */
import type { BotAvatarType } from "bot-avatars";

type BotOptions = {
  type: BotAvatarType;
  state: "default" | "working";
  size: number | string;
  paused: boolean;
  label: string;
};

type Island = {
  render: (o: BotOptions) => void;
  unmount: () => void;
};

let lib: Promise<(node: HTMLElement) => Island> | null = null;

function load() {
  lib ??= Promise.all([import("react"), import("react-dom/client"), import("bot-avatars")]).then(
    ([React, ReactDOM, { BotAvatar }]) =>
      (node: HTMLElement): Island => {
        const root = ReactDOM.createRoot(node);
        return {
          render: (o) =>
            root.render(
              React.createElement(BotAvatar, {
                type: o.type,
                state: o.state,
                size: o.size,
                paused: o.paused,
                "aria-label": o.label,
              }),
            ),
          unmount: () => root.unmount(),
        };
      },
  );
  return lib;
}

export function bot(node: HTMLElement, options: BotOptions) {
  let current = options;
  let island: Island | null = null;
  let gone = false;
  void load()
    .then((make) => {
      if (gone) return;
      island = make(node);
      island.render(current);
    })
    .catch(() => {});
  return {
    update(next: BotOptions) {
      current = next;
      island?.render(next);
    },
    destroy() {
      gone = true;
      // React warns when a root unmounts during a render; the next microtask is safe.
      const i = island;
      island = null;
      if (i) queueMicrotask(() => i.unmount());
    },
  };
}
