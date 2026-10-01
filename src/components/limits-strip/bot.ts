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
      const i = island;
      island = null;
      if (i) queueMicrotask(() => i.unmount());
    },
  };
}
