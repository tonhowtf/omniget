# OmniGet 平台提取器移植 — 交接文档

> 日期：2026-07-01  
> 状态：Phase 0-4 基本完成；14/15 平台 Downloader 已移植到 `omniget-core`，Bilibili auth 网络流程已移入 core，桌面写入通过 runtime provider 适配  
> Fork：https://github.com/gkd2323c/omniget  
> PR：https://github.com/tonhowtf/omniget/pull/163

---

## 1. 背景

OmniGet 的原生平台提取器（YouTube、Twitter、Bilibili 等）原本全部位于 `src-tauri/src/platforms/`，与 Tauri GUI 框架强耦合。这导致：

- CLI 工具（`omniget-cli`）无法复用这些提取器，只能降级走通用 yt-dlp 路径
- 桌面应用换框架（如从 Tauri 换成 egui、终端 UI）需要全部重写
- 无法在服务端/无 GUI 场景复用下载逻辑

**目标**：将平台提取器从 Tauri 层迁移到 `omniget-core`，使其成为框架无关的纯 Rust 库。

---

## 2. 架构设计

### 2.1 移植前后对比

```
移植前（Tauri 专属）
├── src-tauri/
│   ├── src/
│   │   ├── platforms/          ← 所有提取器都在这里
│   │   │   ├── youtube/mod.rs
│   │   │   ├── twitter/mod.rs
│   │   │   └── ...
│   │   ├── models/media.rs     ← Tauri 特有模型
│   │   ├── core/               ← 重新导出 omniget_core::
│   │   └── cookies/             ← Tauri 特有 Cookie 管理
│   └── Cargo.toml

移植后（核心库 + Tauri 适配层）
├── src-tauri/
│   ├── omniget-core/
│   │   ├── src/
│   │   │   ├── platforms/      ← 提取器在这里（框架无关）
│   │   │   │   ├── mod.rs         (registry + re-exports)
│   │   │   │   ├── traits.rs      (PlatformDownloader trait)
│   │   │   │   ├── generic_ytdlp.rs
│   │   │   │   ├── youtube.rs
│   │   │   │   ├── twitter.rs     (未移植)
│   │   │   │   └── ...
│   │   │   ├── models/media.rs   ← 核心模型（与 Tauri 共享）
│   │   │   └── core/             ← 下载引擎、HTTP、编解码
│   │   └── Cargo.toml
│   └── src/
│       ├── platforms/          ← 重新导出核心库（pub use）
│       └── ...
```

### 2.2 PlatformDownloader Trait

所有平台提取器实现同一个 trait：

```rust
// omniget-core/src/platforms/traits.rs
#[async_trait]
pub trait PlatformDownloader: Send + Sync {
    fn name(&self) -> &str;
    fn can_handle(&self, url: -> bool;
    async fn get_media_info(&self, url: &str) -> Result<MediaInfo>;
    async fn download(&self, info: &MediaInfo, opts: &DownloadOptions, 
                      progress: Sender<ProgressUpdate>) -> Result<DownloadResult>;
}
```

### 2.3 共享模型

`MediaInfo`、`DownloadOptions`、`DownloadResult`、`MediaType`、`VideoQuality` 定义在 `omniget-core/src/models/media.rs`，Tauri 层通过 `pub use omniget_core::models::media::*` 重新导出，零重复。

---

## 3. 已移植平台清单

### 3.1 核心基础设施

| 组件 | 文件 | 说明 |
|------|------|------|
| PlatformDownloader trait | `omniget-core/src/platforms/traits.rs` | 统一下载接口 |
| CookieProvider trait | `omniget-core/src/platforms/cookie_provider.rs` | 框架无关 Cookie 路径/手动 Cookie 抽象，CLI 默认读取 app data cookies.txt |
| GenericYtdlpDownloader | `omniget-core/src/platforms/generic_ytdlp.rs` | yt-dlp 通用封装，含质量选择、HLS、直链、fallback |
| Platform 枚举 | `omniget-core/src/platforms/mod.rs` | URL→平台检测 |
| is_direct_file_url | `omniget-core/src/platforms/mod.rs` | 直链文件检测 |

### 3.2 平台提取器

| 平台 | 文件 | 大小 | 特殊依赖 |
|------|------|------|----------|
| YouTube | `youtube.rs` | 16KB | yt-dlp 引擎、播放列表支持 |
| Vimeo | `vimeo.rs` | 6KB | yt-dlp |
| Douyin | `douyin.rs` | 9KB | yt-dlp + referer |
| Bluesky | `bluesky.rs` | 14KB | GenericYtdlp |
| Twitch | `twitch.rs` | 12KB | GenericYtdlp |
| Pinterest | `pinterest.rs` | 11KB | cookie + referer |
| Reddit | `reddit.rs` | 28KB | cookie + 音频提取 |
| Instagram | `instagram.rs` | 36KB | cookie + GraphQL API |
| TikTok | `tiktok.rs` | 24KB | cookie + 浏览器指纹 |
| Twitter/X | `twitter.rs` | 41KB | CookieProvider + GraphQL guest token + yt-dlp fallback |
| P2P | `p2p.rs` + `p2p_words.rs` | 16KB | 纯网络传输 |
| DirectFile | `direct_file.rs` | 5KB | HTTP 直链下载 |

---

## 4. 未移植平台分析

### 4.1 Twitter（~41KB） — Phase 3 ✅ core/CLI/桌面注册已完成

已完成：
- `omniget-core/src/platforms/cookie_provider.rs` 定义 `CookieProvider`
- `omniget-core/src/platforms/twitter.rs` 已移除 Tauri 依赖
- CLI 核心平台注册表已接入 `TwitterDownloader`
- 桌面端 `src-tauri/src/lib.rs` 已切换注册 `omniget_core::platforms::TwitterDownloader`
- 桌面端已实现 `DesktopCookieProvider`，委托 `crate::cookies::account_path_for_consumer` 与设置里的手动 cookie
- 原生 Twitter 失败时仍保留 yt-dlp fallback
- 2026-07-01 实测：`omniget-cli --json --proxy http://127.0.0.1:7897 info https://twitter.com/CTVJLaidlaw/status/1600649710662213632` 命中 `platform=twitter`，GraphQL 200，提取 2 个媒体项
- 2026-07-01 实测：同 URL `download -o target/twitter-smoke` 成功下载 2 个 mp4（16,344,602 + 14,872,817 bytes）

实测修复：
- DefaultCookieProvider 从全局 `cookies.txt` 读取时必须按 domain 过滤 Netscape cookie，否则会把整份 cookies 拼进 Cookie header，Windows 下触发 yt-dlp `os error 206`
- Twitter 多媒体下载分支不能把 progress 发到无人消费的 mpsc channel，否则 direct downloader 会在缓冲区满后死锁

待收尾：
- `src-tauri/src/platforms/twitter/` 旧适配层已不再注册，可后续单独删除
- 桌面 GUI 运行时 smoke test 尚未执行；当前完成的是 `cargo check` 层面的注册切换验证

### 4.2 Bilibili（~200KB/36 文件） — Phase 4 基本完成

已迁移到 `omniget-core/src/platforms/bilibili/`：
- 底座：`api.rs`, `wbi.rs`, `url_kind.rs`, `cdn.rs`, `cover.rs`, `cookie.rs`
- 解析：`parser/` 全量子模块（video / bangumi / cheese / favlist / list / space / popular / history / watch_later / festival）
- 媒体选择与元数据：`preview.rs`, `naming.rs`, `nfo.rs`
- 弹幕：`danmaku/`（proto/xml/json/ass）
- 下载执行层：`engine/`（fetch/query/mux）
- 入口与 fallback：`mod.rs`, `legacy.rs`, `notify.rs`

同步完成的抽象与接线：
- `CookieProvider` 增加 `cookie_path_for_account(domain, slug)`，支持 Bilibili 精确账户 cookie 读取
- 默认 `CookieProvider` 支持 core/CLI 写入 `cookies/<domain>/<slug>.txt`
- 新增 `BilibiliRuntimeProvider`，用于注入 active account、settings、session-expired 通知
- 桌面端实现 `DesktopBilibiliRuntimeProvider`，保留原 Bilibili active-account 选择语义
- Bilibili auth（二维码、短信、CAPTCHA、账号探测）已迁移到 `omniget-core/src/platforms/bilibili/auth/`
- `BilibiliRuntimeProvider::persist_account` 负责登录 cookie 持久化，桌面端实现仍复用现有 `cookies::storage` registry
- 桌面 registry 已从 `platforms::bilibili::BilibiliDownloader` 切到 `omniget_core::platforms::BilibiliDownloader`
- CLI registry 已注册 core `BilibiliDownloader`
- `omniget-core` 新增依赖：`once_cell`, `thiserror`, `md-5`, `hex`, `hmac`, `murmur3`, `qrcode`

当前验证：
- `cargo check -p omniget-core` ✅
- `cargo check -p omniget-cli` ✅
- `cargo check` ✅
- `cargo test -p omniget-core --lib platforms::bilibili` ✅ 51 passed
- `cargo test -p omniget-core --lib` ✅ 244 passed
- 2026-07-01 继续移植 auth 后复验：`cargo check -p omniget-core` ✅，`cargo check` ✅，`cargo test -p omniget-core --lib platforms::bilibili` ✅ 51 passed，`cargo test -p omniget-core --lib` ✅ 244 passed

仍未迁移 / 阻塞：
- 旧 `src-tauri/src/platforms/bilibili/auth/` 仍保留但桌面命令已改用 core auth；可在确认无其他引用后删除
- 旧 `src-tauri/src/platforms/bilibili/` 下载相关模块仍保留，供 cleanup 过渡使用；删除需要单独清理引用
- GUI runtime smoke test 尚未执行

建议下一步：
1. 跑桌面 GUI Bilibili 登录账号下载 smoke test，确认 core Downloader 与桌面 runtime provider 行为一致
2. 删除旧 Tauri `platforms/bilibili/auth/`，确认 `bilibili_auth` 命令仍只依赖 core auth
3. 清理旧 Tauri Bilibili 下载模块，只保留必要 adapter 或完全移除旧目录

### 4.3 Magnet — 视需求决定

**阻塞依赖**：`librqbit` crate、`crate::core::trackers`

**可选方案**：
- 将 `librqbit` 加为 `omniget-core` 的可选 dependency
- 或保留在 Tauri 层，CLI 不支持磁力下载

---

## 5. 移植模式（标准操作程序）

每个平台的移植遵循相同模式：

```bash
# 1. 在核心库创建新文件
cp src-tauri/src/platforms/<name>/mod.rs src-tauri/omniget-core/src/platforms/<name>.rs

# 2. 替换第一行
sed -i 's/use omniget_core::models::progress::ProgressUpdate;/use crate::models::progress::ProgressUpdate;/' \
  src-tauri/omniget-core/src/platforms/<name>.rs

# 3. 在核心库 platforms/mod.rs 注册
echo "pub mod <name>;" >> src-tauri/omniget-core/src/platforms/mod.rs
echo "pub use <name>::<Name>Downloader;" >> src-tauri/omniget-core/src/platforms/mod.rs

# 4. 更新 Tauri 重新导出
# 在 src-tauri/src/platforms/mod.rs 中：
#   - 删除 `pub mod <name>;`
#   - 添加 `pub use omniget_core::platforms::<Name>Downloader;`

# 5. 更新 lib.rs 引用
# 将 `platforms::<name>::<Name>Downloader` 改为 `omniget_core::platforms::<Name>Downloader`

# 6. 编译测试
cargo check -p omniget-core
cargo check  # 全 workspace
```

### 关键注意点

1. **`pub use` 不要立即删除**：先添加核心库导出，确认编译通过后再清理旧模块
2. **Tauri 特有依赖**：如遇到 `crate::cookies::*` 或 `crate::storage::*`，需要引入 trait 抽象
3. **测试移植**：`#[cfg(test)]` 模块随文件一起搬运，确保单元测试不中断
4. **文件布局**：小平台使用单文件（如 `youtube.rs`）；Bilibili 这种多子系统平台使用目录模块（`bilibili/mod.rs` + 子模块）

---

## 6. 测试指南

### 6.1 编译检查

```bash
# 仅核心库
cargo check -p omniget-core

# 全 workspace（含 Tauri）
cargo check

# CLI
cargo build -p omniget-cli --release
```

### 6.2 单元测试

```bash
# 核心库全部测试
cargo test -p omniget-core --lib

# 各平台子模块测试
cargo test -p omniget-core --lib platforms::youtube::tests
cargo test -p omniget-core --lib platforms::instagram::tests
```

当前：核心库完整测试 **244 tests passed**；Bilibili 子模块定向测试 **51 tests passed**。

### 6.3 CLI 实操测试

```bash
# 构建
cargo build -p omniget-cli --release

# YouTube 信息
omniget-cli --proxy http://127.0.0.1:7897 --json info \
  'https://www.youtube.com/watch?v=dQw4w9WgXcQ'

# YouTube 下载
omniget-cli --proxy http://127.0.0.1:7897 --json download \
  'https://www.youtube.com/watch?v=dQw4w9WgXcQ' \
  -q 1080 -o ~/Downloads

# Cookie 导入
omniget-cli import-cookies ~/Downloads/cookies.txt --dry-run
omniget-cli import-cookies ~/Downloads/cookies.txt -n bilibili

# 批量下载
omniget-cli --proxy http://127.0.0.1:7897 batch urls.txt -m 3
```

### 6.4 回归测试

每次移植新平台后，确认：
- [ ] `cargo check` 无新增错误
- [ ] `cargo test -p omniget-core --lib` 无新增失败
- [ ] 桌面 app 启动正常、下载队列工作正常
- [ ] CLI info/download 对已移植站点正常

---

## 7. 构建产物

| 产物 | 路径 | 说明 |
|------|------|------|
| 核心库（静态） | `target/release/omniget_core.lib` | 平台提取器 + yt-dlp + HTTP |
| 桌面应用 | `target/release/omniget.exe` | GUI + 所有平台 |
| CLI 工具 | `target/release/omniget-cli.exe` | 头less + 核心库平台 |

---

## 8. PR 建议

建议将当前 PR 拆分为两个：

**PR-A（已 ready）**：Phase 0-2 基础设施 + 12 个平台移植
- 安全、已测试、不破坏现有功能
- 包含 `import-cookies` 新命令
- 包含 `omniget-cli` binary

**PR-B（后续）**：Bilibili smoke + auth/旧模块清理
- Twitter 已完成 core/CLI/桌面注册切换，旧 `src-tauri/src/platforms/twitter/` 可单独删除
- Bilibili Downloader 与 auth 已完成 core/CLI/桌面注册切换，仍需 GUI smoke 与旧模块删除

---

## 9. 已知问题

| 问题 | 状态 | 说明 |
|------|------|------|
| P2P 代码验证 | ✅ 已修复 | 使用 `super::p2p_words` 替代 `crate::platforms::p2p_words` |
| Magnet 磁力链 | ⏸ 暂缓 | 需 librqbit 进入核心库 |
| CLI 平台调度器 | ✅ 已实现 | `info` / `download` 通过核心 PlatformRegistry 路由，非 generic 失败时回退 GenericYtdlp |
| Twitter CLI 实测 | ✅ 已通过 | info + carousel download 通过，测试产物位于 `src-tauri/target/twitter-smoke/` |
| Twitter 桌面注册 | ✅ 已切换 | `lib.rs` 注册 core Twitter，并在 setup 安装 `DesktopCookieProvider`；`cargo check` 通过 |
| Bilibili Downloader 迁移 | ✅ 已切换 | core 已包含 cookie/legacy/notify/mod，CLI 与桌面 registry 已注册 core Bilibili，51 个定向测试通过 |
| Bilibili auth/旧模块清理 | ⏳ 部分完成 | auth 网络流程已迁入 core，桌面写入由 provider 适配；旧目录暂保留，GUI runtime smoke 未跑 |

---

## 10. 下一步

如果要继续 Phase 3 清理：

1. 删除未注册的 `src-tauri/src/platforms/twitter/` 旧适配层
2. 运行桌面 GUI smoke test，确认队列中的 Twitter 下载仍走 core Twitter
3. 视情况优化 Twitter 多文件下载的聚合进度显示

如果要继续 Bilibili：
1. 跑桌面 GUI 登录账号下载 smoke test，确认 core Downloader 与 `DesktopBilibiliRuntimeProvider` 行为一致
2. 删除旧 `src-tauri/src/platforms/bilibili/auth/`，确认桌面 auth 命令仍编译通过
3. 清理旧 `src-tauri/src/platforms/bilibili/` 下载相关代码，只保留必要 adapter 或完全移除旧目录

---

*文档维护者：Hanako  
最后更新：2026-07-01*
