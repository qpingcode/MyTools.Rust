# MyTools Rust

一个尽量简单的 Rust + Slint 桌面启动器，面向 Windows 和 macOS。

## 运行

Windows 需要 Rust stable（MSVC 工具链）、Visual Studio C++ Build Tools 和 Windows SDK。

```powershell
.\scripts\debug-run.ps1
# 发布版本
.\scripts\debug-run.ps1 -Release
# 只编译和放置插件，不打开窗口
.\scripts\debug-run.ps1 -BuildOnly
# 启动时直接显示窗口，便于开发
.\scripts\debug-run.ps1 -Show
# 编译 release 并更新本机安装（默认 C:\soft\MyTools）
.\scripts\deploy.ps1
```

macOS 需要 Rust stable 和 Xcode Command Line Tools：

```bash
bash scripts/debug-run.sh
bash scripts/debug-run.sh --release
bash scripts/debug-run.sh --build-only
bash scripts/debug-run.sh --show
```

脚本编译整个 workspace，并把插件放入 `target/debug/plugins/`（发布版为 `target/release/plugins/`）。
启动器位于对应目录的 `mytools.exe`（macOS 为 `mytools`），此后可以直接运行。
Windows 已进行本机编译和交互验证；macOS 代码和 CI 已配置，仍需在 Mac 上进行原生运行验证。

## 使用

- 顶部输入名称或别名搜索应用和插件。
- 启动后隐藏主窗口并显示托盘图标，`Alt+空格` 全局唤出并全选已有搜索内容，直接输入即可替换；macOS 对应 `Option+空格`。
- `↑` / `↓` 或鼠标单击选择结果，`Enter` 或鼠标双击执行结果的第一个操作；启动器中 `Esc` 隐藏到托盘，保留搜索内容和插件进程。
- Launch、Run as admin 和 Open in browser 调用成功后自动隐藏主窗口；失败时保留窗口显示错误。
- Windows 单击托盘图标显示窗口；托盘菜单提供“显示 MyTools”“语言”“开机启动”和“退出”。macOS 点击菜单栏图标后选择“显示 MyTools”。关闭主窗口也会隐藏到托盘。
- 从托盘选择“退出”才会结束启动器并关闭搜索插件。
- 若 `Alt+空格` 被其他程序占用，会显示主窗口和错误信息，避免程序启动后无法访问。插件快捷键注册失败、配置文件有误、插件启动失败等问题显示在屏幕右下角的提示框中：它不抢焦点，可以手动关闭，鼠标移开 12 秒后自动关闭。
- 同一安装目录的 MyTools 只能运行一个：再次启动（双击、开始菜单、开机自启）只会把已运行的窗口叫出来，然后自行退出。不同目录的副本（如 `target/` 下的开发版和安装版）互不影响。托盘切换语言和 `deploy.ps1` 重启时，新进程带 `--restart` 启动，最多等 10 秒让旧进程退出。
- MyTools 启动时不等待插件：搜索框立即可用，插件准备好后自动加入搜索。还有插件在加载时，底栏显示“已加载 n/m 个插件，结果可能不完整”。
- 拖动搜索图标、搜索输入框四周的留白、底栏空白或普通状态文字移动窗口；输入框内可正常编辑和选择文字，底部两个按钮仅处理点击。插件可以拖动标题区域。
- 搜索按匹配强弱排序，从强到弱：与标题完全相同、标题开头、某个词的开头、按顺序匹配多个词的开头（如 `cw`、`cwp`、`cp`、`cargop` 都能找到 CargoWise PRS，`jsb` 找到记事本）、标题中间包含、忽略空格和标点后包含、零散的字母。驼峰（CargoWise → Cargo、Wise）、字母与数字之间、每个汉字都算词的分界，汉字也可以用拼音全拼或首字母输入。
- 标题和副标题中匹配的文字以强调色（`theme.accent`）显示，高亮的就是决定排序的那次匹配；有多种匹配方式时选跳过词最少、最紧凑的一种。零散的字母匹配、只靠关键词或插件内部字段匹配的结果不高亮。
- 底部左侧按钮执行当前结果的首个动作，右侧“操作”按钮或 `Ctrl+K` 打开其余动作列表（macOS 为 `⌘K`），不重复首个动作。没有其他动作时“操作”按钮置灰，快捷键也不会打开菜单。列表内用 `↑` / `↓` 选择，`Enter` 执行；`Esc` 先关闭列表，再按一次隐藏窗口。点击列表外部只关闭列表；输入新搜索词也会关闭列表。
- 文件、文件夹和快捷方式类型的结果（Everything、剪贴板中的单个文件、应用程序的快捷方式等）可以直接拖到其他窗口：拖到文件资源管理器或桌面会复制一份（不会移走原文件），拖到 Teams、邮件中作为附件，拖到 VS Code 中打开。按住结果移动几像素即开始拖动，拖动时显示该文件的系统图标。商店应用没有磁盘路径，不能拖动。
- 应用程序插件在 Windows 扫描用户和公共开始菜单、桌面下的 `.lnk` / `.url`，并通过系统 AppsFolder 列出没有快捷方式的商店/MSIX 应用（如 Claude、Windows Terminal），这类应用只能“打开”，没有路径可复制或定位；macOS 扫描 `/Applications`、`/System/Applications`、`~/Applications` 下的 `.app`，最多返回 50 条匹配结果。
- 按 `Alt+V` 全局打开翻译，自动进入 `tr ` 模式并聚焦输入框；macOS 对应 `Option+V`。可在设置窗口中修改或清空，保存后立即生效。
- 翻译和格式化使用编辑视图：顶部输入的内容同步到左侧多行输入框，在左侧继续编辑不会改动顶部；右侧显示结果，可以选中复制。`Ctrl+Enter` 执行（翻译、复制），顶部输入框中按 `↓` 进入左侧输入框。
- 输入问题后按 `Tab` 询问 AI：进入 `ai ` 对话并直接发送；输入框为空时按 `Tab` 开始一个新对话。回答逐字显示，
  `Enter` 继续提问、`Shift+Enter` 换行，回答进行中按 `Esc` 停止；`Ctrl+K` 复制回答、开始新对话或打开最近的对话。
  需要时 AI 会联网搜索（Tavily 或必应）并标注来源，也可以接入其他 MCP 服务。默认使用 DeepSeek，
  需要环境变量 `DEEPSEEK_API_KEY`（与翻译相同），配置见 [实用插件](docs/utility-plugins.md)。
- 命令“就选中文字询问 AI”（默认无快捷键，可在设置中指定）：把其他应用中选中的文字作为附件开始对话，回答可替换原文。
- 按 `Ctrl+Shift+Alt+V` 把剪贴板内容以纯文本粘贴到当前应用，不打开 MyTools 窗口（见“剪贴板历史”）。
- 在 Teams、网页、Word 等应用中选中文字后按 `Ctrl+Alt+T`（命令“翻译选中文字”，仅 Windows）：选中的文字在输入框中时，
  直接用译文替换；不可编辑（如网页正文、聊天记录）时打开 `tr ` 显示原文和译文。
- 插件在配置中声明的命令可以直接搜索，例如输入“纯文本”找到“以纯文本粘贴剪贴板”；结果说明里显示命令所属插件和快捷键。
- 输入 `> `（脚本插件的别名，后面要有空格）列出所有脚本；`> podman ps` 这样的任意命令，第一个结果是“在终端运行”，按 `Enter` 在新的 PowerShell 窗口执行（优先使用 `PATH` 中的 PowerShell 7，加载用户配置文件，窗口保留输出）。别名或名称与输入完全相同的脚本排在它前面。
- 输入 `1+1`，计算器直接在主列表返回 `2`；按 `Enter` 复制结果。
- 输入 `calc 1+1` 或 `calc (2+3)*4`，仅显示计算器返回的结果。
- 输入别名后必须有空白才进入指定插件搜索；单独输入 `calc` 仍属于普通搜索。

主窗口可拖动边缘或四角调整大小（最小 560×320），移动或调整后自动记住大小和位置，退出重启后恢复。显示器被移除、分辨率或多屏布局变化时：窗口仍部分可见则缩小到不超过该屏幕并完整拉回屏幕内，完全不可见则居中到主屏幕；保存的尺寸异常时使用默认尺寸。大小和位置保存在配置文件同目录的 `window-position.json`。

## 日志

MyTools 把自己和各插件的运行情况写入 `mytools.exe` 同目录的 `logs/mytools.log`，每行一条 JSON 记录，包含时间、级别（`debug`、`info`、`warn`、`error`）、来源（`mytools` 或插件 ID）、事件代码、内容和可选的详情。日志内容统一用英文，不随界面语言变化。文件超过 2 MB 时转存为 `mytools.log.1`（更旧的依次后移），共保留 10 份。

- 记录的内容：MyTools 启动和退出；插件进程启动、就绪及用时；初始化失败、启动超时、启动过程中退出；运行中崩溃（含退出码）、心跳无响应、搜索或动作卡住及随后的自动重启；反复崩溃被停止；手动重启；设置修改后重新加载插件；快捷键注册失败、配置文件有误；插件搜索出错和超时。
- 启动用时：窗口可用且随启动的插件都已就绪（或失败）时，记一条 `startup.done`（info），列出总用时、
  MyTools 各步骤完成的时刻（读取配置、创建窗口、加载图标、加载插件、注册快捷键、事件循环开始）以及每个插件
  从开始启动到就绪的用时和创建进程的用时，最慢的插件排在前面。超过 5 秒时改记 `startup.slow`（warn）；
  启动 5 秒后仍未结束时先记一条 `startup.waiting`（warn），写明还在等哪些插件，即使启动最终卡住也能看到原因。
- 不记录搜索词，以免把剪贴板内容等私人文字写进日志。
- 只有 MyTools 主程序写日志文件。插件用 `mytools_plugin_api::log` 的 `info`、`warn`、`error` 等函数写日志，它们输出到 stderr，由主程序收集并标上插件 ID；插件直接打印到 stderr 的普通文字也会被收集，记为警告。
- `config.json` 的 `log_level` 设置记录的最低级别，默认 `info`；设为 `debug` 会额外记录全局搜索中插件的超时。
- 在设置窗口的 **日志** 页可以查看、筛选和复制日志，见 [docs/utility-plugins.md](docs/utility-plugins.md) 的“设置”一节。

### 崩溃日志

MyTools 和各插件发生 panic、未处理的原生异常（如访问冲突），或主程序因错误退出时，会把时间、进程名、PID、出错位置、错误信息和调用栈追加到 `logs/crash.log`。日志超过 1 MB 时转存为 `crash.log.1`，最多保留两份。插件通过 `MYTOOLS_LOG_DIR` 继承宿主的日志目录。每次崩溃同时在 `mytools.log` 中记一条错误，详情里是完整的报告。栈溢出由 Rust 运行时直接终止进程，不会记录。

## 多语言

新增的书签、编码/解码、格式化、进程管理、UUID、翻译、快捷文本及保持唤醒插件，输入和配置方式见
[实用插件说明](docs/utility-plugins.md)。

支持简体中文和英文，默认跟随操作系统语言。启动脚本首次运行会从 `config.example.json` 创建程序旁的 `config.json`，以后保留已有配置。

```json
{
  "language": "auto"
}
```

`language` 可设为 `auto`、`zh-CN` 或 `en`，修改后退出托盘中的 MyTools 并重新启动生效。开发时的配置在 `target/debug/config.json`，发布版在 `target/release/config.json`；也可以通过 `MYTOOLS_CONFIG` 指定配置文件路径。不存在配置时使用 `auto`；配置无法读取或格式错误时使用系统语言，并在状态栏显示错误。

也可以在托盘的“语言”子菜单中选择“跟随系统”“简体中文”或“English”：保存成功后提示重启，选择“立即重启”会关闭搜索插件、释放快捷键并重新启动；选择“稍后”、按 Esc 或关闭提示窗口，会保留当前程序，下次启动应用新语言。菜单标记已保存的语言选择，Windows 使用 Lucide `check`，与开机启动菜单一致。保存失败会显示错误；已有配置中的其他字段会保留。

系统语言为 `zh`、`zh-CN`、`zh-SG` 或 `zh-Hans` 时使用简体中文，其他语言目前回退英文。目标语言缺失的文案回退英文；插件名称最终回退 `name.defaultValue`。应用名、文件名、用户输入和插件别名保持原样。

- `crates/launcher/locales/*.json`：主界面、托盘、状态和通用 Action 文案，编译时嵌入。
- `crates/core/locales/*.json`：核心诊断信息，编译时嵌入；core 的 Action 只提供标题 key 和可选自定义标题。
- `plugins/<插件>/locales/*.json`：插件独立维护的翻译。宿主用 `name.key` 查找插件名称，构建脚本会将这些文件放入插件目录。
- `crates/plugin-api/src/i18n.rs`：不依赖 UI 的语言解析、JSON 字典、英文回退及 `{name}` 参数替换。
- `ui/i18n.slint`：纯翻译回调，由每个可执行程序接入自己的字典。

宿主启动搜索插件或独立窗口插件时传入 `MYTOOLS_LOCALE=zh-CN` 或 `en`。插件可通过 `mytools_plugin_api::i18n::language()` 获取语言，负责自己的结果说明、自定义 Action 标题和窗口文案；宿主按原文显示插件自定义标题。计算器的表达式内容无需翻译。

宿主和插件的翻译随可执行文件打包，修改其 JSON 后需要重新编译；插件显示名称从插件目录下的语言文件读取，修改后重启生效。语言切换通过重启生效，当前不提供设置页。

## 开机启动

在托盘中点击“开机启动”注册当前可执行文件，再次点击则移除。Windows 开启时菜单项显示 Lucide `check` 图标，关闭时隐藏。默认关闭，不在程序启动时自动写入系统配置，也不需要管理员权限；登录启动时隐藏主窗口并显示托盘。菜单读取系统注册状态，不依赖配置中的布尔值。Windows 对 Slint 1.18.1 忽略托盘菜单图片的问题做了原生适配；适配不可用及 macOS 下保留文字 `✓`。

- Windows：当前用户 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `MyTools.Rust` 值，路径加引号，不带 `--show`。
- macOS：`~/Library/LaunchAgents/com.mytools.rust.plist`，使用 `RunAtLoad`，下次登录生效，不立即启动第二个实例。

移动程序后需要在新位置重新开启。建议在固定安装目录使用；操作系统的启动项管理仍可能阻止登录启动。macOS 仍需原生验证。

## 结构

```text
crates/
  core/               不依赖 UI 的插件发现、进程管理、搜索路由和通用动作
    src/lib.rs        核心公开接口
    src/commands.rs   插件命令：搜索结果与全局快捷键绑定
  launcher/           桌面程序
    src/main.rs       程序入口与 Slint 生成代码
    src/app.rs        UI 状态与搜索结果更新
    src/actions.rs    动作面板与执行协调
    src/search.rs     后台搜索与输入防抖
    src/desktop.rs    全局快捷键、窗口激活和拖动
    src/desktop/drag.rs 把文件结果拖到其他应用
    src/commands.rs   按快捷键直接执行插件命令
    src/chat.rs       对话视图：显示对话、发送输入、应用插件推送的回答
    ui/main.slint     主窗口组合
    ui/components/   搜索框、结果项、状态栏
    ui/tray.slint     Slint 原生托盘菜单
    build.rs          构建时编译 Slint UI
  plugin-api/         搜索插件共用的 JSON 请求、结果和标准输入输出循环
  ai/                 调用对话模型：OpenAI 兼容接口（整段与流式回复、工具调用）、MCP 客户端、DeepSeek 默认值、环境变量与 JSONC 配置
plugins/
  ask-ai/             Ask AI：对话视图中与模型对话，Tab 进入；内置联网搜索 MCP 服务、对话历史
  calculator/         无独立窗口的计算器插件
  applications/       无独立窗口的快捷方式扫描与应用搜索插件
  settings/           设置窗口：插件别名、快捷键与启用状态
  crash-test/         故意出错的测试插件：启动崩溃、运行崩溃、心跳延迟等，见 docs/utility-plugins.md
    src/scan.rs       平台默认目录、目录遍历和去重
    src/index.rs      名称索引、匹配与结果构造
  clipboard/          剪贴板监听、持久化和历史操作插件
ui/
  theme.slint         共享配色
  components/         共享按键提示、按钮等基础组件
  assets/             SVG 图标，构建时嵌入
scripts/              debug-run（编译并从 target 运行）、deploy（更新本机安装）、打包
.github/workflows/    Windows 构建、测试和发布配置
```

依赖方向为 `launcher → core → plugin-api`；UI 不参与插件计算或搜索路由。
应用或插件专用界面放在自己的 `ui/`，根目录 `ui/` 仅共享设计组件，无需单独的 Rust crate。
单元测试保留在被测 Rust 文件底部。Slint 的生成代码由 `include_modules!` 引入，不提交到源码目录。

独立窗口插件使用自己的 Slint 界面；无界面的搜索插件通过标准输入输出交换 JSON，不依赖 UI 框架，也不使用动态库或额外服务。

## 插件配置

示例见 `plugins/calculator/plugin.json` 和 `plugins/applications/plugin.json`。
配置不再包含 `protocolVersion` 和 `capabilities`。

| 字段 | 用途 |
| --- | --- |
| `id` | 唯一标识 |
| `name.defaultValue` | 缺少翻译时的显示名称；`name.key` 对应插件 `locales/<语言>.json` 中的翻译 key |
| `version` | 插件版本 |
| `type` | `window`：独立窗口，点击结果时启动；`search`：无界面搜索插件，进程常驻（何时启动见 `startup`）。省略时默认 `window` |
| `entry` | 相对当前插件目录的可执行文件路径；示例使用无扩展名入口，Windows 自动补 `.exe`，macOS 保持原名。不支持参数或跳出目录 |
| `icon` | 内置 Lucide 名称、文本，或系统图标引用 `{"native":"绝对路径"}` |
| `alias` | 插件别名，不包含空白；`别名 + 空白 + 搜索词` 只搜索指定插件 |
| `search.global` | `true` 时参与主列表普通搜索；`false` 时普通搜索不调用该插件，只能通过别名进入 |
| `search.tab` | 可选，`true` 时在全局搜索中按 Tab 进入该插件（带上已输入的文字），见[插件视图约定](docs/plugin-views.md)的对话视图 |
| `search.aliasTimeoutMs` | 可选，通过别名单独搜索该插件时的等待上限（毫秒），默认 3000，范围 1000–15000。普通（全局）搜索固定 1 秒，不受此值影响 |
| `view` | `list`（默认）、`list-detail`、`editor`（左侧多行输入、右侧单条结果）或 `chat`（对话），见[插件视图约定](docs/plugin-views.md)；只在该插件的搜索作用域内启用 |
| `hotkey` | 可选全局唤起快捷键，进入插件第一个别名的搜索作用域；用户可在设置窗口中修改 |
| `commands` | 可选，插件命令：可被搜索、可绑定全局快捷键，按快捷键时直接执行而不打开窗口，见“插件命令” |
| `contributes.actions` | 可选，按规则为其他结果追加本插件的动作，见“为其他结果追加动作” |
| `startup` | 可选，`eager`：随 MyTools 启动；`lazy`：第一次用到时才启动。省略时 `search.global` 为 `true` 的插件随 MyTools 启动，其他按需启动。需要在后台持续工作的插件（如剪贴板历史）应写 `eager`。可在设置窗口中为每个插件修改 |
| `startupTimeoutMs` | 可选，插件完成初始化的时限（毫秒），覆盖 `config.json` 的 `plugin_startup_timeout_ms`，范围 1000–120000 |

快捷键写成 `Ctrl+Shift+V` 的形式：修饰键为 `Ctrl`、`Shift`、`Alt`、`Win`（macOS 为 `Cmd`），
主键为字母、数字、`F1`–`F20`、`Space`、`Enter`、方向键等，不区分大小写；除功能键外至少需要一个修饰键。

### 插件命令

`search` 类型插件可以在 `commands` 中声明命令。宿主直接从配置读取命令，不需要启动插件就能搜索和注册快捷键：

```json
"commands": [{
  "id": "paste-plain",
  "title": { "key": "command.paste_plain", "defaultValue": "Paste Clipboard as Plain Text" },
  "keywords": ["plain", "纯文本", "无格式"],
  "icon": "builtin:type",
  "hotkey": "Ctrl+Shift+Alt+V"
}]
```

| 字段 | 用途 |
| --- | --- |
| `id` | 命令标识，插件内唯一，只含字母、数字、`-`、`_` |
| `title` | 显示名称，`key` 对应插件 `locales/<语言>.json` 中的翻译 |
| `keywords` | 可选，除名称外还能找到该命令的词 |
| `icon` | 可选，内置图标；省略时使用插件图标 |
| `hotkey` | 可选，默认全局快捷键；用户可在设置窗口中修改或清空 |
| `input` | 可选，`"selection"` 表示命令处理前台应用中选中的文字；`"selection-optional"` 相同，但没有选中文字时也执行（`item` 为空）；默认不带输入 |

- 普通搜索会同时匹配命令的名称、关键词和所属插件名称。插件的别名范围（如 `cb `）只显示插件自己的结果，不列出命令。
- 设置窗口（“打开设置”）和脚本管理窗口（“管理脚本”）都是命令：在全局搜索中输入“设置”“脚本”找到它们；设置插件因此不常驻，第一次打开设置时才启动。
- 从搜索结果执行命令，或按命令的快捷键，宿主都向插件发送 `{"action":{"command":"<id>"}}`（不带 `item`），
  按需启动尚未运行的插件，并按插件返回的 `effect` 复制、粘贴。
- 按快捷键执行时不显示 MyTools 窗口：宿主记住按键时的前台应用，`paste` 粘贴到该应用；失败时在右下角提示。
  命令超过 0.4 秒仍未完成时，在当前窗口所在屏幕的中下方显示“正在<命令>…”，不抢焦点，完成后消失。
- `"input": "selection"` 的命令（目前仅 Windows）：宿主先读取前台应用中选中的文字，作为 `item` 发送给插件。
  插件把结果放在 `effect.copy_text` 并设置 `paste: true`；选中的文字可编辑时宿主粘贴结果替换它，
  否则打开插件的第一个别名范围并填入原文（插件没有别名时只复制结果）。动作同时带 `editable` 说明选中的文字能否替换；
  插件返回 `effect.open: true` 时宿主直接打开插件的别名范围（如 Ask AI 就选中文字开始对话）。
  宿主优先通过 UI Automation 读取选中文字，读不到时（如 VS Code、Notepad++）松开快捷键后发送 `Ctrl+C`，
  用完后恢复原来的剪贴板文本；这时选中的文字会出现在剪贴板历史中。终端窗口不发送 `Ctrl+C`。

启动器读取自身目录下 `plugins/*/plugin.json`，也可通过 `MYTOOLS_PLUGIN_DIR` 指定插件根目录。
添加插件只需在该目录中新建文件夹，放入配置和编译后的程序，然后重启启动器。
插件以自己的文件夹为工作目录启动。配置中的其他字段目前会被忽略。

## 界面配置

所有颜色、字号和尺寸集中在 [ui/theme.slint](ui/theme.slint) 的 `Theme` 全局中，默认值比早期版本整体缩小
（搜索框 18px、标题 14px、正文 13px、说明 12px、结果行高 52px）。可在 `config.json` 的 `theme` 中按属性名覆盖，
颜色用 `#rrggbb` / `#rrggbbaa`，尺寸为逻辑像素，修改后重启生效；未知键或无效值会在状态栏提示：

```json
{
  "theme": {
    "font-search": 18, "font-title": 14, "font-body": 13, "font-small": 12, "font-caption": 11,
    "row-height": 52, "icon-box": 30, "icon-size": 20,
    "accent": "#94ded1", "background": "#1f1f1f"
  }
}
```

## Everything 文件搜索

`everything` 插件通过 Everything 1.4 的 IPC（WM_COPYDATA）直接查询正在运行的 Everything，无需 SDK DLL 或 `es.exe`。
输入 `file 关键字` 搜索（`global: false`，不参与普通搜索），支持 Everything 搜索语法，例如 `file *.sln`、`file 报告 ext:pdf`。
结果为文件/文件夹资源，使用系统图标，提供打开、复制路径、在文件夹中显示，以及下面 IDE 插件的额外动作。
Everything 未运行时提示启动；已安装时按 Enter 启动 Everything。
别名搜索最多等待 5 秒（`aliasTimeoutMs: 5000`）。Everything 正在更新索引而响应过慢时显示“Everything 正忙 — 按 Enter 重试”；
只拿到部分结果时，在列表末尾追加“结果可能不完整 — 按 Enter 重试”。继续输入会取消正在进行的旧查询。

## IDE 打开（dev-open）

`dev-open` 插件在 `plugin.json` 中声明 `contributes.actions`（`when.type: ["file", "folder"]`），宿主会把这些动作追加到**所有**文件/文件夹结果的操作面板（`Ctrl+K`）：

- **用 Rider 打开**：从当前目录向上递归查找 `.sln` / `.slnx` / `.slnf`；整条路径都找不到解决方案时，再向上查找
  `.csproj` / `.fsproj` / `.vbproj` / `.vcxproj` / `.esproj` / `.proj`；仍找不到则提示无法打开。同一目录有多个时优先 `.sln`、
  与目录同名者，再按名称排序。
- **用 VS Code 打开**：打开当前目录；若是文件，则打开其所在目录并定位到该文件。
- **用 Visual Studio 打开**：查找逻辑同 Rider，通过 `vswhere` 选择版本最高的 `devenv.exe`。

在搜索框直接输入一个已存在的绝对路径（或 `ide 路径`）也会列出这三个动作。Rider 依次从 JetBrains Toolbox 脚本、
`%LOCALAPPDATA%\JetBrains\Installations`、`%LOCALAPPDATA%\Programs`、`%ProgramFiles%\JetBrains` 和 PATH 查找；
自动检测失败时可在 `plugins/dev-open/config.json` 中填写 `rider`、`vscode`、`visualstudio` 的程序绝对路径。

### 为其他结果追加动作

任何 `search` 类型插件都可以在 `plugin.json` 中用规则把自己的动作追加到其他结果上：

```json
"contributes": {
  "actions": [
    {
      "when": { "type": ["file", "folder"], "source": ["everything", "clipboard"] },
      "actions": [
        { "command": "rider", "title": { "key": "action.rider", "defaultValue": "Open in Rider" }, "icon": "builtin:square-code" }
      ]
    }
  ]
}
```

- `when.type`：结果类型 `url`、`file`、`folder`、`text`、`application`、`plugin_item`，与结果的 `type` 一致。
- `when.source`：产生该结果的插件 id。
- 字段之间为“且”，列表内为“或”；省略或为空表示不限。多条规则分别匹配，命中的动作按插件加载顺序追加到结果自身动作之后。
- 未知字段或类型值会使 manifest 加载失败，错误显示在状态栏。`actions: []` 的提示类结果不追加任何动作。

执行时宿主向贡献动作的插件发送 `{"action":{"command":"<command>","item":"<值>"}}`，`item` 为结果的主值：
文件/文件夹/应用为路径，`url` 为链接，`text` 为文本，`plugin_item` 为其 `id`。插件返回 `effect.dismiss: true` 表示成功后隐藏启动器。

## 剪贴板历史

`clipboard` 是随 MyTools 启动的独立常驻插件，默认不参与普通全局搜索。
运行期间自动记录文本（包含可读取的 HTML 格式）、图片以及文件列表。
相同内容与格式只保留一条并置顶；图片保存原始分辨率 PNG，额外生成缩略图预览；
文件列表保存路径，不复制文件本身。退出 MyTools 后停止监听。
列表图标按内容区分：普通文本 `type`、链接 `link-2`、颜色值 `palette`、路径文本 `folder-search`、
HTML/代码 `code-xml`、图片 `image`；单个文件或文件夹显示其系统图标（如 Word、PDF 图标），多个文件显示 `files`。
单个文件或文件夹的记录以真实的 `file`/`folder` 类型返回，因此也会出现其他插件追加的动作（如“用 Rider 打开”）；
多个文件、文本和图片仍为 `plugin_item`。

- `Ctrl+Shift+V` 打开历史，也可搜索 `clip ` 或 `clipboard `。
- `Ctrl+Shift+Alt+V`（命令“以纯文本粘贴剪贴板”）不打开窗口，直接把剪贴板中的文本去掉格式（HTML、RTF 等）后粘贴到当前应用。
  粘贴后剪贴板里保留这份纯文本；历史中仍是原来带格式的那一条，不会多出重复记录。剪贴板中没有文本（如只有图片或文件）时在右下角提示。
- 左侧搜索和选择历史，右侧预览完整文本、文件路径或图片；进入插件或修改查询时读取历史，
  展示期间不定时更新。删除或清空完成后重新查询一次。
- `Enter` 或双击执行“粘贴到原应用”；`Ctrl+K` 提供粘贴、纯文本粘贴（文本项）、复制、删除、清空历史。
- Windows 使用剪贴板变更通知，macOS 每 100ms 检测 `NSPasteboard.changeCount`。
  启动时也会读取当前内容；Windows 兼容 PixPin 等截图工具提供的 `CF_DIB` / `BI_BITFIELDS` 位图。
  剪贴板占用时会短暂重试；不支持的格式、过大内容或明确标记为不允许历史记录的内容不会保存。
  极快连续覆盖的中间内容可能在系统允许读取前已经消失。
- 直接粘贴会先恢复剪贴板，隐藏窗口并返回唤起前的应用，等待修饰键释放后发送粘贴快捷键。
  若焦点已被切到其他应用则停止发送。macOS 需用户自行在系统设置中授予 MyTools 辅助功能权限；
  Windows 普通权限进程无法向更高权限窗口模拟输入，此时仍可手动粘贴已恢复的内容。

两个快捷键都可以在设置窗口中修改或清空，保存在 `plugin-settings.json`：

```json
{
  "plugins": {
    "clipboard": {
      "hotkey": "Ctrl+Alt+V",
      "commands": { "paste-plain": { "hotkey": "" } }
    }
  }
}
```

`hotkey` 为空字符串表示不注册。早期版本写在 `config.json` 的 `plugin_shortcuts` 会在启动时迁移到这里，并从 `config.json` 中删除。

历史默认位于 Windows 的 `%LOCALAPPDATA%/MyTools/clipboard`，
macOS 的 `~/Library/Application Support/MyTools/clipboard`；可用 `MYTOOLS_CLIPBOARD_DIR` 覆盖。
内容只保存在本机，当前为明文 JSON、文本和 PNG。清空/删除历史会移除对应内容和预览文件。
移除插件并重启可以停止记录，已有历史保留在数据目录。

数据目录中的 `settings.json` 可配置保留策略，修改后重启生效：

```json
{
  "max_history_days": 10,
  "max_history_count": 500,
  "max_content_bytes": 67108864,
  "max_total_bytes": 268435456
}
```

默认保留 10 天、最多 500 条，按历史内容总量约 256 MiB 淘汰旧记录；
单张图片原始 RGBA 上限 64 MiB，单条文本/HTML 元数据上限 2 MiB，单次文件列表最多 1024 项。
启动、新增及每小时执行过期清理。写入先落临时文件再替换索引，失败时保留原历史；
历史目录使用进程锁限制为单个写入者，避免重复启动时互相覆盖。

插件界面采用宿主提供的声明式 `list-detail` 约定，接口与示例见 [插件视图约定](docs/plugin-views.md)。

## 应用程序插件

快捷方式扫描由独立常驻搜索插件 `applications` 提供，本体仅发现插件、聚合结果并执行通用动作，
不再扫描系统目录或维护应用索引。插件只依赖 `plugin-api`，没有 Slint 或 core 依赖。
移除该插件目录后，应用扫描功能随之移除，其他插件照常工作。

- 默认参与全局搜索，直接输入应用名称即可搜索，不设置 alias。
- Windows 扫描 `%APPDATA%` 和 `%ProgramData%` 下的 `Microsoft/Windows/Start Menu`，
  以及 `%USERPROFILE%/Desktop`、`%PUBLIC%/Desktop` 中的 `.lnk` 和 `.url`。
- macOS 扫描 `/Applications`、`/System/Applications`、`~/Applications` 中的 `.app`，
  不进入应用包内部。
- 后台建立索引，每 30 秒刷新；初次索引完成前可能暂时没有结果，稍后再次输入即可。
  不能访问的目录会跳过，不跟随目录符号链接；重叠扫描目录去重，已删除的结果立即过滤。
- 支持不区分大小写的多词名称匹配、去除空格和标点后的名称匹配，以及单词首字母，
  例如 `visualstudio`、`vsc` 匹配 `Visual Studio Code`；名称前缀匹配优先，最多返回 50 条。
- 可在启动 MyTools 前设置 `MYTOOLS_APPLICATION_DIRS` 覆盖默认目录；
  Windows 用分号分隔，macOS 用冒号分隔，路径必须为绝对路径。空值表示不扫描目录。

插件结果使用 `Resource::Application`，通过 `{"native":"应用或快捷方式的绝对路径"}`
引用系统原生图标，提取失败回退 Lucide `app-window`。
启动、管理员启动、复制路径和在文件夹中显示仍由宿主通用动作处理。
本次迁移参考 C# 插件的职责边界、后台索引及桌面扫描范围；
尚未迁移其拼音匹配、按启动目标去重或设置页。

## 搜索插件

```json
{
  "id": "calculator",
  "name": { "defaultValue": "计算器" },
  "version": "0.1.0",
  "type": "search",
  "entry": "mytools-calculator",
  "alias": ["calc"],
  "search": { "global": true }
}
```

每个 `search` 插件只运行一个常驻进程，后续搜索复用它。每次请求和响应各占一行 JSON，响应后必须刷新输出。
普通搜索将原搜索词发送给所有 `global: true` 且已就绪的搜索插件并聚合结果；指定别名时移除别名前缀，仅调用该插件，也不混入应用快捷方式。
`global: false` 的插件不会收到普通搜索请求，默认在第一次通过别名、快捷键或动作用到时才启动（见 `startup`）。

请求：

```json
{"query":"1+1"}
```

响应：

```json
{"items":[{"title":"2","description":"1+1 = 2","resource":{"type":"text","text":"2"}}]}
```

通过别名搜索时，请求带有 `"scoped":true`，表示用户点名搜索该插件，别名之后输入的任何内容都应回答；全局搜索时没有这个字段，插件只应回答自己认得的内容，免得出现在每次普通搜索里。

没有匹配结果时返回 `{"items":[]}`。`title` 为主列表标题，`description` 为可选说明，`resource` 描述该结果的资源。旧的 `copy_text` 已由 `resource` 替代。

### 启动与初始化

插件进程启动后，宿主发送的第一个请求是 `{"init":{}}`。插件准备好接受搜索时回答 `{"ready":true}`；无法工作时回答 `{"error":"简短原因"}`。回答之前可以输出任意条 `{"progress":"正在扫描开始菜单"}` 说明进度，它们不算响应。

```text
→ {"init":{}}
← {"progress":"正在打开数据库"}
← {"ready":true}
→ {"query":"1+1","timeoutMs":1000}
← {"items":[...]}
```

使用 `mytools-plugin-api` 的插件不用处理这些消息：`serve` / `serve_interactive` 会自动回答 `ready`。需要先完成准备工作的插件在调用 `serve` 前用 `on_init` 注册一个初始化函数，在其中可调用 `progress` 报告进度，返回 `Err` 表示失败。耗时的索引应在初始化之后放到后台进行，先用部分结果回答搜索。不认识 `init` 的旧插件会把它当作空搜索回答，只要回答中没有 `error` 也视为就绪。

插件依次经过这些状态，设置窗口用颜色圆点显示，鼠标悬停可看到原因：

| 状态 | 圆点 | 含义 |
| --- | --- | --- |
| 运行中 | 绿 | 已回答 `ready`，正常接受搜索 |
| 启动中 | 黄 | 进程已启动，尚未回答 `init`；悬停可看到插件报告的进度 |
| 按需启动 | 空心灰 | `lazy` 插件还没有被用到，进程未启动 |
| 已停止 | 红 | 初始化失败、启动过程中退出、超过启动时限，或就绪后反复崩溃（60 秒内超过 5 次）；可在设置中重启 |
| 已停用 | 灰 | 在设置中关闭 |

- 启动时限默认 10 秒，可用 `config.json` 的 `plugin_startup_timeout_ms` 修改，插件可用 `startupTimeoutMs` 单独设置。超时的插件被结束进程并暂时停用（不写入设置），右下角弹出提示；之后可在设置窗口重启它，下次启动 MyTools 时照常加载。
- 就绪之前不发送搜索和心跳。全局搜索跳过未就绪的插件；通过别名搜索时，未就绪的插件显示“正在加载…”，就绪后自动重新搜索；已停止或已停用的插件显示原因。
- 就绪的插件崩溃、心跳无响应或请求卡住时，宿主自动重启它，状态回到“启动中”。
- 宿主把各插件的状态写入 MyTools 目录下的 `plugin-status.json`（退出时删除），设置窗口据此显示；设置窗口通过 `plugin-requests.json` 请求重启插件。

### 内置图标

宿主内置 73 个 Lucide SVG，随程序编译打包，运行时无需联网或额外字体。
插件在 `plugin.json` 中设置默认图标：

```json
"icon": "builtin:calculator"
```

搜索插件也可为每条结果指定 `icon`，例如：

```json
{"items":[{"title":"2","icon":"builtin:circle-check","resource":{"type":"text","text":"2"}}]}
```

单条结果省略 `icon`、传入空字符串或未知的 `builtin:` 名称时，回退到插件图标；
插件默认图标未指定或无效时使用 `builtin:puzzle`。MyTools 自身的结果按资源类型使用
`app-window`、`file`、`folder`、`globe`、`file-text` 图标。
不带 `builtin:` 前缀的字符串继续作为原样文本显示；图标名称不参与翻译。

常用名称包括 `calculator`、`plus`、`search`、`settings`、`copy`、`folder-open`、
`external-link`、`square-terminal`、`code`、`star` 和 `triangle-alert`。
完整支持列表见 [names.txt](ui/assets/icons/names.txt)，引用时加 `builtin:` 前缀。
内置名称作为插件协议保持稳定，上游更新时保留已有名称。

SVG 统一通过根目录 `ui/components/icon.slint` 显示，颜色跟随主题；
资源固定为 Lucide 1.66.0，版权与许可证见 [Lucide LICENSE](ui/assets/icons/LICENSE)。
运行脚本会将许可证复制到输出目录的 `licenses/Lucide.txt`，分发程序时一并保留。
维护时使用 PowerShell 7 执行 `./scripts/update-lucide.ps1` 下载固定版本资源并生成 Slint 映射，
日常构建不下载资源。当前支持内置图标、文本和系统原生图标，尚不支持任意图片文件。

Rust 插件构造 `SearchItem` 时，设置 `icon: None` 继承插件图标，或设置
`icon: Some("builtin:folder".into())` 覆盖它。旧 JSON 响应无需增加字段。

### 系统原生图标

插件结果和 `plugin.json` 的 `icon` 都支持系统图标引用：

```json
"icon": { "native": "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Editor.lnk" }
```

macOS 示例为 `"icon": { "native": "/Applications/Editor.app" }`。
Rust 使用 `Icon::Native { native: path }`。路径必须为当前平台的绝对路径；
相对路径及含空字符的路径回退默认图标。

插件仅传路径，不传图像数据，也不依赖 Slint；系统 API 和图片缓存属于 launcher 的显示适配。
Windows 使用 Shell 的系统图标列表提取 `.lnk`、程序或文件图标，保留快捷方式自定义图标并省略链接覆盖标识；macOS 使用
`NSWorkspace.iconForFile` 获取 `.app` 应用包或文件图标。原生图标保留原始颜色和透明度，
内置 Lucide 图标继续跟随主题着色。

图标在独立工作线程读取，主线程只更新仍引用同一路径的结果行；缓存最多 1024 项，
6 小时后在下次显示时后台重新加载（期间继续显示旧图标），提取失败的路径 1 分钟后重试。加载中、文件缺失或提取失败时显示 Lucide `app-window`，
不会阻塞搜索、清空结果或导致插件停止。

### 结果与操作

通用动作由宿主实现，插件按结果声明资源即可复用，不放在 `plugin.json` 中：

| `resource` | 默认操作（首个为主操作） |
| --- | --- |
| `{"type":"text","text":"2"}` | 复制 |
| `{"type":"url","url":"https://example.com"}` | 在浏览器中打开、复制链接 |
| `{"type":"file","path":"D:\\notes.txt"}` | 打开、复制路径、复制文件名、在文件夹中显示 |
| `{"type":"folder","path":"D:\\notes"}` | 打开文件夹、复制路径、复制文件夹名称、在父文件夹中显示 |
| `{"type":"application","path":"D:\\app.exe"}` | 启动、以管理员身份运行（Windows）、复制路径、在文件夹中显示 |

文件、文件夹和应用路径必须为绝对路径；浏览器链接支持 HTTP/HTTPS。Windows 应用快捷方式的路径操作针对 `.lnk` 本身，按钮明确标为“复制快捷方式路径”等。

结果可通过 `actions` 指定动作及顺序，首个动作同时成为底部主按钮和 `Enter` 的动作：

```json
{
  "title": "报告",
  "resource": { "type": "file", "path": "D:\\report.pdf" },
  "actions": [
    { "id": "copy_name", "title": "复制报告名称" },
    { "id": "open" },
    { "id": "reveal" }
  ]
}
```

省略 `actions` 使用类型默认动作；显式数组完整替换默认动作，`[]` 表示无动作。重复 ID 仅保留第一次，`title` 可覆盖默认名称。不匹配资源类型的动作会使该条结果被拒绝，其他有效结果继续显示。

支持的宿主动作 ID 为 `open`、`open_in_browser`、`copy_text`、`copy_path`、`copy_name`、`reveal`、`run_as_admin`，以及独立可执行程序使用的 `launch`。插件还可使用 `plugin` 动作配合 `command` 和 `item` 声明自己的操作（`item` 为执行时回传的值，未声明则不回传），见视图约定。操作面板绑定打开时的结果快照，新搜索或结果更新会关闭面板。

Windows 应用结果的 `.exe`、`.com` 和 `.lnk` 默认提供 `run_as_admin`，图标为 Lucide `shield`，
可在操作面板中选择。宿主使用 `ShellExecuteExW` 的 `runas` 动词，由系统显示 UAC；
取消授权会显示已取消，不会改用普通权限启动。`.lnk` 在执行时通过 Shell Link 接口读取
可执行目标、完整参数和工作目录，参数不会经命令行解释器拼接；非可执行目标会拒绝。
`.url`、文档和文件夹不提供此动作。显式 `actions` 数组仍完整覆盖默认动作。
macOS 没有通用的 GUI 应用管理员启动语义，不显示该动作；显式声明时返回不兼容错误，
普通 `.app` 仍通过系统 `open` 启动，应用所需授权由应用自身处理。

Rust 插件可复用 `mytools-plugin-api::serve`，只需实现 `query -> Vec<SearchItem>`。
标准输出只用于响应，日志写入标准错误；宿主当前丢弃标准错误。
搜索在后台并行执行，输入有 120ms 防抖。普通（全局）搜索最多等候 1 秒；通过别名单独搜索时按插件的 `search.aliasTimeoutMs` 等待（默认 3 秒，最多 15 秒）。
输入改变时宿主立即停止等待旧搜索，插件运行时也会跳过或提前结束被新查询取代的旧搜索（见下文 `cancelled()`）。
别名搜索超时时列表显示“搜索超时 — 按 Enter 重试”，重试的等待时间翻倍（不超过 15 秒）；全局搜索超时只在状态栏提示。
超时不会终止插件。就绪后崩溃、返回无效响应或被意外关闭的插件会被自动重启：下一次请求立即拉起新进程，
后台看门狗每秒检查一次进程是否存活，并对已就绪的插件每 10 秒发送独立的心跳消息 `{"ping":n}`；插件运行时在读线程中立即回复
`{"pong":n,"busyMs":当前请求已执行的毫秒数}`，即使正在执行搜索也不受影响。3 秒没有 `pong`，
或搜索执行超过 20 秒、动作执行超过 60 秒，视为卡死并重启。同一插件 60 秒内最多重启 5 次，超过后标记为已停止，可在设置中重启。
启动阶段（回答 `init` 之前）不发心跳，由启动时限判断，见“启动与初始化”。
插件可调用 `mytools_plugin_api::cancelled()` 判断当前搜索是否已被更新的查询取代，调用 `deadline()` 获取宿主等待的截止时间。
从托盘退出 MyTools 时同时关闭搜索插件；隐藏启动器不会停止插件，已经打开的独立窗口插件继续独立运行。

计算器支持 `+ - * /`、括号、正负号和小数。表达式不完整、无效或除零时不返回结果，数值使用 `f64`。

## 渲染与许可

锁定 Slint 1.18.1，运行时启用 Winit + FemtoVG，不引入 GPUI、WebView 或 Skia 后端。
Release 开启 thin LTO 和符号裁剪。实际启动耗时和内存需在对应平台用 Release 构建测量，不能用二进制大小代替运行内存。

UI 使用 [Slint](https://slint.dev/)。托盘不显示“关于”；主窗口按 `F1` 可查看包含官方 `AboutSlint` 组件的许可说明。第三方依赖遵循各自许可。

## 检查

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## Windows 便携版打包

GitHub Actions 目前只有 **Windows** 工作流，在推送和 Pull Request 时运行，也可手动 **Run workflow**。它检查格式、Clippy 并执行测试，通过后构建 Release 便携包。macOS 工作流已暂时移除，macOS 版本需在 Mac 上本地构建（见下文）。
完成后进入该次运行的 **Artifacts**，下载 `MyTools-windows-x64-portable`，其中包含便携版 ZIP 和 SHA256 校验文件。
解压 ZIP 后直接运行 `MyTools/mytools.exe`，通过托盘或 `Alt+空格` 打开窗口，无需安装 Rust。
请保留整个目录结构：程序依赖旁边的 `plugins/`，不能只复制主程序。

本地打包：

```powershell
.\scripts\package-windows.ps1
```

打包输出为 `dist/MyTools-windows-x64-portable.zip`，包含 Release 主程序、所有插件、语言文件、默认配置、说明和图标许可。
便携版表示免安装运行；剪贴板、快捷文本和搜索历史仍存放在用户数据目录，开机启动仍使用系统启动项。
翻译插件仍需自行设置 `DEEPSEEK_API_KEY` 环境变量。压缩包不包含本机配置、历史、密钥或调试符号。

## GitHub Releases 与 macOS DMG

推送 `v*` 版本标签后，Windows 检查通过后会自动创建或更新对应的 GitHub Release，并上传
`MyTools-windows-x64-portable.zip` 及 SHA256。macOS DMG 目前不由 CI 生成，需在 Mac 上运行
`python3 scripts/package-macos.py` 本地打包。

打开 DMG，把 `MyTools.app` 拖到“应用程序”目录，再双击运行。应用包已包含全部插件和图标。
macOS 配置保存到 `~/Library/Application Support/MyTools/config.json`，避免写入应用包而破坏签名。
当前使用临时签名（ad-hoc），尚未进行 Developer ID 签名和 Apple 公证，首次打开可能被 Gatekeeper 拦截。
普通分支推送和 Pull Request 只上传 Actions Artifacts，不创建 Release。

本地 macOS 打包：`python3 scripts/package-macos.py`。
