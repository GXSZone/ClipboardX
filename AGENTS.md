# ClipboardX 工程规范与架构宪章 (`AGENTS.md`)

> 本文件为 ClipboardX 仓库的最高工程与 UI 设计契约。所有开发、重构、测试与代码审查必须严格遵守。

---

## 1. 产品定位

- **目标平台**：Windows / macOS / Linux（Windows 与 macOS 体验优先；Linux X11 全功能支持，Wayland 原生协议优先 + 优雅降级）。
- **核心原则**：纯离线单机、MIT 开源、极速无感直贴、双库全量透明加密（`history.db` + `vault.db`）、系统原生 + 按需插件化 OCR、零后台遥测。

---

## 2. 六边形架构与严格 DAG 依赖

```text
ClipboardX/
├── Cargo.toml                  # Workspace 配置与强制 lint
├── rustfmt.toml                # 统一格式化配置
├── locales/                    # 编译期嵌入 i18n (zh-CN.yml / en-US.yml)
├── assets/                     # 图标、品牌资源与内嵌字体
└── crates/
    ├── clipboardx-model/       # [SSOT] 领域模型、强类型枚举、ThemeTokens、配置 Schema、Port Traits、领域错误
    ├── clipboardx-core/        # 纯业务逻辑：内容解析、多维检索(FTS5/拼音/正则/模糊)、去重淘汰、.cbx 归档
    ├── clipboardx-storage/     # 加密持久化：SQLCipher 双库、CAS XChaCha20 Blob、缩略图缓存、密钥环与信封加密
    ├── clipboardx-platform/    # 三端原生桥：剪贴板监听、隐私门禁、热键状态机、焦点恢复与模拟连填 (unsafe 唯一隔离区)
    ├── clipboardx-ocr/         # OCR 适配器：系统原生 + 在线/本地插件管理 + 子进程池
    ├── clipboardx-updater/     # 可选更新：Releases 签名清单、Ed25519 验签、SHA-256 校验与原子替换
    └── clipboardx-app/         # GPUI 桌面端（按功能垂直组织：shell / history / vault / settings / theme）
```

**依赖方向（严禁反向或循环依赖）**：
- `clipboardx-model` 不依赖任何内部 crate。
- `clipboardx-storage`、`clipboardx-platform`、`clipboardx-ocr`、`clipboardx-updater` 仅依赖 `clipboardx-model`。
- `clipboardx-core` 仅依赖 `clipboardx-model` 与 `clipboardx-storage`。
- `clipboardx-app` 作为顶层装配者协调上述所有 crate。

---

## 3. 单一事实来源（SSOT）与工程原则

1. **SSOT 唯一定义**：`FilterTab`、`SmartSubtype`、`ShortcutAction`、`ThemeTokens`、`AppConfig` 与 `DomainError` 均唯一定义在 `clipboardx-model`，严禁在其他 crate 复制枚举值、魔法字符串或颜色常量。上层一律使用穷尽 `match` 消费。
2. **务实抽象**：仅对多平台/多后端端口定义 Trait（`PlatformBridge`、`OcrProvider`、`SecretStore`、`UpdateVerifier`）；单一实现（`HistoryStore`、`VaultStore`、`CbxArchive`、`ContentInspector`）直接编写具体 `struct`，禁止无意义套壳。
3. **一步到位重构**：直接破坏式替换旧类型与旧字段，严禁保留过渡 shim、废弃别名或死代码。
4. **极简依赖**：纯算法逻辑（颜色空间与 WCAG 对比度、URL 拆解、时间戳格式化、Base64/JWT 解码、单实例锁等）采用零重依赖的标准 Rust 实现。

---

## 4. 代码质量门禁（Rust 2024）

- **强制配置**：`edition = "2024"`，`rust-version = "1.89"`。
- **生产代码 Lint**：`clippy::unwrap_used = "deny"`、`clippy::expect_used = "deny"`、`clippy::panic = "deny"`、`clippy::todo = "deny"`、`clippy::unimplemented = "deny"`。
- **内存安全**：全仓 `unsafe_code = "forbid"`（仅 `clipboardx-platform` 的 OS FFI 最小作用域例外，且每块必须附带 `// SAFETY: ...` 不变式说明）；密钥与口令字节统一使用 `zeroize::Zeroizing<...>`；剪贴板临时写回与还原操作必须封装 RAII Guard。
- **错误处理与日志**：统一使用 `thiserror` 强类型错误与 `?` 传播；`tracing` 日志严禁输出剪贴板明文、账号或密码。
- **注释规范**：注释只说明代码明确意图与非显而易见的原因，不夹杂无关信息。
- **提交前门禁**：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 必须全绿无警告，禁止通过 `#[allow(...)]` 绕过。
- **测试组织**：单元测试内联于源码底部 `#[cfg(test)] mod tests`；集成/契约测试置于各 crate 的 `tests/` 目录。

---

## 5. UI 像素级规范（SSOT）

### 5.1 零描边色阶美学
- 非图标元素 **100% 零描边（`0px border`）**，仅通过 **4 级表面色阶（`bg/window` → `bg/surface` → `bg/elevated` → `bg/code`）**、圆角嵌套与双层环境投影建立层次。

### 5.2 窗口尺寸与布局参数 (`px`)

| 视图形态 | 尺寸 | 圆角 | 核心参数 |
| :--- | :--- | :--- | :--- |
| **双栏主面板** | `680 × 420` | `14` | 顶栏 `38`；筛选条 `29`；左列 `246`（row 高 `40`, radius `8`, 标块 `22×22` radius `6`）；右预览卡 `410`（radius `10`）；底栏 `27` |
| **简洁单栏** | `328 × 420` | `14` | 顶栏 `36`；筛选条 `29`；单列 row `312 × 40`；底栏 `27` |
| **设置中心** | `680 × 420` | `14` | 左侧导航 `164`（项高 `30`, radius `7`）；右侧内容区 `492`（radius `10`, 内嵌卡片 radius `9`） |
| **侧边动画** | `680 ↔ 328` | `14` | `180ms`，`cubic-bezier(0.22, 1.0, 0.36, 1.0)`，记忆上次展开态 |
| **悬浮投影** | — | — | 主投影 `offset(0, 18) blur 40 spread -6`（Night `40%` / Light `28%`）；贴合投影 `offset(0, 4) blur 10`（Night `16%` / Light `10%`） |

### 5.3 双模式主题色表

| Token | Night | Light | 用途 |
| :--- | :--- | :--- | :--- |
| `bg/window` | `#14171D` | `#EFF2F6` | 悬浮窗外壳背景 |
| `bg/surface` | `#1B1F27` | `#FFFFFF` | 预览卡 / 设置内容区表面 |
| `bg/elevated` | `#232833` | `#E6EAF0` | 胶囊按钮 / 设置卡片 / 键帽 |
| `bg/hover` | `#2B313E` | `#DCE1EA` | 悬停态底色 |
| `bg/selected` | `#223452` | `#DDEAFF` | 列表/导航选中高亮 |
| `bg/code` | `#111419` | `#F3F5F9` | 代码井 / 预览井 / 分段底槽 |
| `text/primary` | `#F1F5F9` | `#0F172A` | 主标题、正文、主值 |
| `text/secondary` | `#94A3B8` | `#475569` | 副标题、次要文字 |
| `text/muted` | `#64748B` | `#7C8A9E` | 时间、来源、说明 |
| `accent/blue` | `#3B82F6` | `#2563EB` | 主强调色、主动作按钮 |
| `accent/blueSoft` | `#1B2F4D` | `#DBEAFE` | 激活筛选标签、信息徽章底 |
| `accent/green` | `#22C55E` | `#16A34A` | 加密状态、有效提示 |
| `accent/greenSoft` | `#122D21` | `#DCFCE7` | 绿色柔底胶囊 |
| `accent/amber` | `#F59E0B` | `#D97706` | 账号 / 警告徽章强调 |
| `accent/amberSoft` | `#312512` | `#FEF3C7` | 琥珀柔底胶囊 |
| `accent/purple` | `#A78BFA` | `#7C3AED` | 图片 / OCR 徽章强调 |
| `accent/purpleSoft` | `#261E42` | `#EDE9FE` | 紫色柔底胶囊 |
| `accent/red` | `#EF4444` | `#DC2626` | 危险 / 删除 / 错误提示 |

### 5.4 字体与排版
- **字体**：UI `Noto Sans SC`（或系统回退字体）+ 等宽 `Fira Code`。
- **字号**：弹窗主标题 `15px Bold`；搜索输入 `13px`；列表标题 `12px Medium`；副标题/元标签/按钮 `10px~11px`；小徽章 `9px Bold`；色卡主值 `18px Bold`。
