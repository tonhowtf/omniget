import * as api from "./api";

type Phase = "empty" | "reading" | "ready" | "error";

class ImageSource {
  phase = $state<Phase>("empty");
  path = $state<string | null>(null);
  result = $state<api.Extraction | null>(null);
  error = $state<string | null>(null);
  count = $state(6);
  private seq = 0;

  name = $derived(this.path ? (this.path.split(/[\\/]/).pop() ?? this.path) : "");

  async read(path: string) {
    const id = ++this.seq;
    this.path = path;
    this.phase = "reading";
    this.error = null;
    try {
      const out = await api.extract(path, this.count);
      if (id !== this.seq) return;
      this.result = out;
      this.phase = "ready";
    } catch (e) {
      if (id !== this.seq) return;
      this.error = api.errorText(e);
      this.phase = "error";
    }
  }

  async setCount(count: number) {
    this.count = count;
    if (this.path && this.phase !== "reading") await this.read(this.path);
  }

  clear() {
    this.seq++;
    this.phase = "empty";
    this.path = null;
    this.result = null;
    this.error = null;
  }
}

export const imageSource = new ImageSource();
