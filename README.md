# MyTools Rust

一个尽量简单的 Rust + Slint 桌面启动器，面向 Windows 和 macOS。

## 运行

Windows 需要 Rust stable（MSVC 工具链）、Visual Studio C++ Build Tools 和 Windows SDK。

```powershell
.\run.ps1
# 发布版本
.\run.ps1 -Release
# 只编译和放置插件，不打开窗口
.\run.ps1 -BuildOnly
# 启动时直接显示窗口，便于开发
.\run.ps1 -Show
```

macOS 需要 Rust stable 和 Xcode Command Line Tools：

```bash
bash run.sh
bash run.sh --release
bash run.sh --build-only
bash run.sh --show
```

脚本编译整个 workspace，并把插件放入 `target/debug/plugins/`（发布版为 `target/release/plugins/`）。
启动器位于对应目录的 `mytools.exe`（macOS 为 `mytools`），此后可以直接运行。
Windows 已进行本机编译和交互验证；macOS 代码和 CI 已配置，仍需在 Mac 上进行原生运行验证。

## 使用

- 顶部输入名称或别名搜索应用和插件。
- 启动后隐藏主窗口并显示托盘图标，`Alt+空格` 全局唤出并聚焦搜索框；macOS 对应 `Option+空格`。
- `↑` / `↓` 或鼠标单击选择结果，`Enter` 或鼠标双击执行结果的第一个操作；启动器中 `Esc` 隐藏到托盘，保留搜索内容和插件进程。
- Launch、Run as admin 和 Open in browser 调用成功后自动隐藏主窗口；失败时保留窗口显示错误。
- Windows 单击托盘图标显示窗口；托盘菜单提供“显示 MyTools”“语言”“开机启动”和“退出”。macOS 点击菜单栏图标后选择“显示 MyTools”。关闭主窗口也会隐藏到托盘。
- 从托盘选择“退出”才会结束启动器并关闭搜索插件。
- 若 `Alt+空格` 被其他程序占用或托盘初始化失败，会显示主窗口和错误信息，避免程序启动后无法访问。
- 拖动搜索图标、搜索输入框四周的留白、底栏空白或普通状态文字移动窗口；输入框内可正常编辑和选择文字，底部两个按钮仅处理点击。插件可以拖动标题区域。
- 底部左侧按钮执行当前结果的首个动作，右侧“操作”按钮或 `Ctrl+K` 打开其余动作列表（macOS 为 `⌘K`），不重复首个动作。没有其他动作时“操作”按钮置灰，快捷键也不会打开菜单。列表内用 `↑` / `↓` 选择，`Enter` 执行；`Esc` 先关闭列表，再按一次隐藏窗口。点击列表外部只关闭列表；输入新搜索词也会关闭列表。
- 应用程序插件在 Windows 扫描用户和公共开始菜单、桌面下的 `.lnk` / `.url`；macOS 扫描 `/Applications`、`/System/Applications`、`~/Applications` 下的 `.app`，最多返回 50 条匹配结果。
- 输入 `1+1`，计算器直接在主列表返回 `2`；按 `Enter` 复制结果。
- 输入 `calc 1+1` 或 `calc (2+3)*4`，仅显示计算器返回的结果。
- 输入别名后必须有空白才进入指定插件搜索；单独输入 `calc` 仍属于普通搜索。

## 多语言

新增的书签、编码/解码、格式化、进程管理、UUID、翻译及快捷文本插件，输入和配置方式见
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
  launcher/           桌面程序
    src/main.rs       程序入口与 Slint 生成代码
    src/app.rs        UI 状态与搜索结果更新
    src/actions.rs    动作面板与执行协调
    src/search.rs     后台搜索与输入防抖
    src/desktop.rs    全局快捷键、窗口激活和拖动
    ui/main.slint     主窗口组合
    ui/components/   搜索框、结果项、状态栏
    ui/tray.slint     Slint 原生托盘菜单
    build.rs          构建时编译 Slint UI
  plugin-api/         搜索插件共用的 JSON 请求、结果和标准输入输出循环
plugins/
  calculator/         无独立窗口的计算器插件
  applications/       无独立窗口的快捷方式扫描与应用搜索插件
    src/scan.rs       平台默认目录、目录遍历和去重
    src/index.rs      名称索引、匹配与结果构造
  clipboard/          剪贴板监听、持久化和历史操作插件
ui/
  theme.slint         共享配色
  components/         共享按键提示、按钮等基础组件
  assets/             SVG 图标，构建时嵌入
run.ps1 / run.sh       编译、放置插件、运行
.github/workflows/    Windows/macOS 构建和测试配置
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
| `type` | `window`：独立窗口，点击结果时启动；`search`：无界面搜索插件，MyTools 启动时加载并常驻。省略时默认 `window` |
| `entry` | 相对当前插件目录的可执行文件路径；示例使用无扩展名入口，Windows 自动补 `.exe`，macOS 保持原名。不支持参数或跳出目录 |
| `icon` | 内置 Lucide 名称、文本，或系统图标引用 `{"native":"绝对路径"}` |
| `alias` | 插件别名，不包含空白；`别名 + 空白 + 搜索词` 只搜索指定插件 |
| `search.global` | `true` 时参与主列表普通搜索；`false` 时普通搜索不调用该插件，只能通过别名进入 |
| `view` | `list`（默认）或 `list-detail`；详情布局只在该插件的搜索作用域内启用 |
| `hotkey` | 可选全局唤起快捷键，进入插件第一个别名的搜索作用域；可通过宿主配置覆盖 |

启动器读取自身目录下 `plugins/*/plugin.json`，也可通过 `MYTOOLS_PLUGIN_DIR` 指定插件根目录。
添加插件只需在该目录中新建文件夹，放入配置和编译后的程序，然后重启启动器。
插件以自己的文件夹为工作目录启动。配置中的其他字段目前会被忽略。

## 剪贴板历史

`clipboard` 是随 MyTools 启动的独立常驻插件，默认不参与普通全局搜索。
运行期间自动记录文本（包含可读取的 HTML 格式）、图片以及文件列表。
相同内容与格式只保留一条并置顶；图片保存原始分辨率 PNG，额外生成缩略图预览；
文件列表保存路径，不复制文件本身。退出 MyTools 后停止监听。

- `Ctrl+Shift+V` 打开历史，也可搜索 `clip ` 或 `clipboard `。
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

宿主 `config.json` 中覆盖快捷键，修改后重启生效；空字符串表示不注册该快捷键：

```json
{
  "language": "auto",
  "plugin_shortcuts": { "clipboard": "Ctrl+Shift+V" }
}
```

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

MyTools 启动时运行每个 `search` 插件一次，后续搜索复用进程。每次请求和响应各占一行 JSON，响应后必须刷新输出。
普通搜索将原搜索词发送给所有 `global: true` 的搜索插件并聚合结果；指定别名时移除别名前缀，仅调用该插件，也不混入应用快捷方式。
`global: false` 的插件同样在启动时加载，但不会收到普通搜索请求。

请求：

```json
{"query":"1+1"}
```

响应：

```json
{"items":[{"title":"2","description":"1+1 = 2","resource":{"type":"text","text":"2"}}]}
```

没有匹配结果时返回 `{"items":[]}`。`title` 为主列表标题，`description` 为可选说明，`resource` 描述该结果的资源。旧的 `copy_text` 已由 `resource` 替代。

### 内置图标

宿主内置 64 个 Lucide SVG，随程序编译打包，运行时无需联网或额外字体。
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

图标在独立工作线程读取，主线程只更新仍引用同一路径的结果行；缓存最多 256 项，
5 分钟后在下次显示时重新加载。加载中、文件缺失或提取失败时显示 Lucide `app-window`，
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

支持的宿主动作 ID 为 `open`、`open_in_browser`、`copy_text`、`copy_path`、`copy_name`、`reveal`、`run_as_admin`，以及独立可执行程序使用的 `launch`。插件还可使用 `plugin` 动作配合 `command` 声明自己的操作，见视图约定。操作面板绑定打开时的结果快照，新搜索或结果更新会关闭面板。

Windows 应用结果的 `.exe`、`.com` 和 `.lnk` 默认提供 `run_as_admin`，图标为 Lucide `shield`，
可在操作面板中选择。宿主使用 `ShellExecuteExW` 的 `runas` 动词，由系统显示 UAC；
取消授权会显示已取消，不会改用普通权限启动。`.lnk` 在执行时通过 Shell Link 接口读取
可执行目标、完整参数和工作目录，参数不会经命令行解释器拼接；非可执行目标会拒绝。
`.url`、文档和文件夹不提供此动作。显式 `actions` 数组仍完整覆盖默认动作。
macOS 没有通用的 GUI 应用管理员启动语义，不显示该动作；显式声明时返回不兼容错误，
普通 `.app` 仍通过系统 `open` 启动，应用所需授权由应用自身处理。

Rust 插件可复用 `mytools-plugin-api::serve`，只需实现 `query -> Vec<SearchItem>`。
标准输出只用于响应，日志写入标准错误；宿主当前丢弃标准错误。
搜索在后台并行执行，输入有 80ms 防抖；每次聚合最多等候 1 秒。超时或返回无效响应的插件会被停止，重启 MyTools 后重新加载。
从托盘退出 MyTools 时同时关闭搜索插件；隐藏启动器不会停止插件，已经打开的独立窗口插件继续独立运行。

计算器支持 `+ - * /`、括号、正负号和小数。表达式不完整、无效或除零时不返回结果，数值使用 `f64`。

目前不含设置页。

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

GitHub Actions 分为 **Windows** 和 **macOS** 两个独立工作流，均在推送和 Pull Request 时运行，也可手动 **Run workflow**。两个平台分别检查格式、Clippy 并执行测试；Windows 检查通过后构建 Release 便携包，macOS 构建 Release 应用并打包 DMG。下载便携版请进入 **Windows** 工作流。
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

推送 `v*` 版本标签后，Windows 和 macOS 各自检查通过后，会自动创建或更新同一 GitHub Release 并上传产物：

- Windows：`MyTools-windows-x64-portable.zip` 及 SHA256。
- macOS：`MyTools-macos-<架构>.dmg` 及 SHA256，架构取决于 macOS runner。

打开 DMG，把 `MyTools.app` 拖到“应用程序”目录，再双击运行。应用包已包含全部插件和图标。
macOS 配置保存到 `~/Library/Application Support/MyTools/config.json`，避免写入应用包而破坏签名。
当前使用临时签名（ad-hoc），尚未进行 Developer ID 签名和 Apple 公证，首次打开可能被 Gatekeeper 拦截。
普通分支推送和 Pull Request 只上传 Actions Artifacts，不创建 Release。

本地 macOS 打包：`python3 scripts/package-macos.py`。
