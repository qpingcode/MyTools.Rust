# 实用插件

运行 `./scripts/debug-run.ps1 -BuildOnly` 或 `bash scripts/debug-run.sh --build-only` 编译并打包全部插件。
别名后输入空格进入插件作用域。默认 Enter 执行首个动作，Ctrl+K（macOS 为 ⌘K）打开其他动作。

| 插件 | 输入示例 | 交互 |
| --- | --- | --- |
| Chrome 书签 | 直接输入书签标题、URL、文件夹关键词 | 全局搜索，不设置 alias；Enter 在系统默认浏览器打开，操作菜单可复制链接 |
| encode | `encode hello`、`encode base64 hello` | 普通列表展示各格式结果；Enter 复制 |
| decode | `decode base64 5Lit5paH` | 普通列表展示各格式结果；Enter 复制 |
| formatter | `fmt {"a":1}`、`fmt @C:\Temp\input.xml` | 编辑视图：左侧粘贴内容，自动识别 JSON 或 XML，右侧显示格式化结果；`Ctrl+Enter` 复制 |
| process-killer | `ps chrome`、`ps :8080`、`ps port 8080`、`ps 8080` | 按进程名称或本地 TCP/UDP 端口查询；Enter 或“结束进程”动作终止所选进程 |
| UUID generator | `uuid `、`uuid v7`、`uuid v4 10` | UUID v4/v7；单个提供大小写、无连字符、花括号格式，批量提供复制全部；最多 50 个 |
| translator | `tr hello`、`tr en 你好`、`tr zh Hello` | 编辑视图：左侧输入原文，`Ctrl+Enter` 发起翻译，完成后右侧显示译文，再按 `Ctrl+Enter` 复制；单词另有音标、英文释义和例句，`Ctrl+K` 可朗读。选中文字后按 `Ctrl+Alt+T` 翻译选中内容 |
| quick text | `qt add email | hello@example.com`，然后 `email` 或 `qt email` | Enter 保存；全局按短语、内容查找；Enter 复制，“删除快捷文本”动作删除 |
| 保持唤醒 | `awake ` | 插件启用期间电脑不会因空闲睡眠，屏幕保持常亮；`awake ` 显示当前状态。在设置中停用插件即恢复正常睡眠 |

encode 按 `base64`、`md5`、`url`、`sha1`、`sha256`、`sha512` 的顺序展示结果。
decode 按 `base64`、`url` 的顺序展示结果。URL 使用 URI component 编码，`+` 保持加号；
哈希不能解码。格式名使用这里的小写值。
decode 严格验证编码与 UTF-8，二进制结果显示错误，不把无效字节替换成乱码。

formatter 仅支持 JSON 和 XML，按内容的第一个字符识别：`{` 或 `[` 为 JSON，`<` 为 XML（开头的空白和 BOM 会被忽略），其他内容提示无法识别。JSON 保留对象键顺序及数字精度。
XML 保留文本内容；含混合文本或 `xml:space="preserve"` 的文档保留原格式，避免改变空白语义。
文本输入上限为 16 KiB，格式化结果上限为 128 KiB。文本工具支持 `@文件路径` 读取多行 UTF-8 输入；
quick text 的 `|` 后也支持文件输入。更新快捷文本时，用相同短语再次执行 add 即可。

## Chrome 数据

自动读取 Windows 的 `%LOCALAPPDATA%\Google\Chrome\User Data`、macOS 的
`~/Library/Application Support/Google/Chrome` 下 `Default`、`Profile *` 的 `Bookmarks`。
每 5 秒后发生查询时重读，不修改 Chrome 文件；损坏或不存在的用户配置会跳过。
可通过 `MYTOOLS_CHROME_DIR` 指定 User Data 根目录。仅展示可打开的 HTTP/HTTPS 书签。

## 翻译配置

密钥只从环境变量 `DEEPSEEK_API_KEY` 读取，配置文件中的 `api_key` 不生效。
优先读取进程环境变量；Windows 未继承该变量时，依次读取用户环境变量和系统环境变量，
因此在 Windows 设置用户变量后可直接重试。插件不把密钥写入文件或日志。
如需修改 API 地址、模型、缓存条数或朗读语音，可将插件目录的 `config.example.json` 复制为 `config.json`；
构建脚本保留已有配置。`config.json` 支持 `//` 和 `/* */` 注释，示例中逐项列出了可选值。
`MYTOOLS_TRANSLATOR_CONFIG` 可指定配置文件路径，`DEEPSEEK_MODEL` 可覆盖模型。

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `base_url` | `"https://api.deepseek.com"` | OpenAI 兼容接口根地址，请求 `<base_url>/chat/completions` |
| `model` | `"deepseek-flash"` | 模型名称 |
| `cache_size` | `100` | 本地保存的最近翻译条数，`0` 关闭缓存 |
| `speech.voice` | `"auto"` | `"auto"` 自动选美式英语语音，或填写语音名称，如 `"Microsoft Zira Desktop"`、macOS 的 `"Samantha"` |
| `speech.rate` | `0` | 语速，`-10`（最慢）到 `10`（最快） |

默认模型与请求格式参考 [DeepSeek 官方文档](https://api-docs.deepseek.com/quick_start)。
配置及环境变量在每次执行翻译时读取；macOS 修改环境变量后需要从带有新环境的进程重新启动 MyTools。
不带方向时，含汉字的文本译成英文，其他文本译成简体中文；可显式使用 `en` 或 `zh`。
输入完成后按 `Ctrl+Enter` 才发送文本；插件等 API 完成后回复，宿主收到回复后再查询一次读取结果。
同一时间只执行一个请求，全程不轮询。
超时、鉴权失败、限流及响应截断会显示“翻译失败，按 Ctrl+Enter 重试”，右侧显示完整错误原因（含所用代理）。

### 翻译选中文字

在其他应用中选中文字后按 `Ctrl+Alt+T`（命令“翻译选中文字”，可在设置中修改，目前仅 Windows）：

- 选中的文字在输入框中（Teams/网页输入框、Word、Outlook 写邮件、VS Code 等）时，译文直接替换选中内容，
  原文前后的空白和换行保持不变。
- 不可编辑（网页正文、Teams 聊天记录、Outlook 阅读窗格等）时打开 `tr ` 编辑视图，左侧为原文，右侧显示译文。

方向与 `tr` 自动模式相同。翻译超过 0.4 秒时，当前窗口所在屏幕的中下方显示“正在翻译选中文字…”及完成后会怎样
（替换选中的文字，或在 MyTools 中显示结果），翻译完成即消失；缓存命中等很快完成的情况不显示。失败时在右下角提示。
译文同样写入缓存。替换时借用剪贴板粘贴，完成后恢复原来的剪贴板文本（图片等非文本内容不恢复），
这次临时内容不进入剪贴板历史。

### 缓存

翻译成功后保存到 `%LOCALAPPDATA%\MyTools\translator\cache.json`（macOS 为
`~/Library/Application Support/MyTools/translator/cache.json`，可用 `MYTOOLS_TRANSLATOR_CACHE` 指定），
保留最近 `cache_size` 条，满了删除最早翻译的一条。再次输入相同内容时直接显示译文，不再请求 API；
`Ctrl+K` →“重新翻译”会重新请求并覆盖缓存。失败结果只保存在内存中。只输入 `tr` 时显示用法说明。

### 单词：音标、朗读、释义与例句

输入单个英文单词（如 `tr hello`），或输入简短中文且译文是单个英文单词（如 `tr 苹果` → apple）时，
同一次请求额外返回该英文单词的信息，在右侧译文下方依次显示（可选中复制）：

1. 译文 — `Ctrl+Enter` 复制；`Ctrl+K` 可朗读该单词（Windows SAPI / macOS `say`，离线）或重新翻译
2. 音标，如 `/həˈləʊ/`
3. 英文释义
4. 两三条例句及中文翻译

DeepSeek 偶尔不按 JSON 格式返回；插件会去掉 Markdown 代码块包裹后再解析，仍不合格时自动重试一次，
再失败则只显示译文。

## 快捷文本数据

数据保存在 Windows 的 `%LOCALAPPDATA%\MyTools\quick-text\phrases.json` 或 macOS 的
`~/Library/Application Support/MyTools/quick-text/phrases.json`，使用文件锁避免多实例覆盖，
写入临时文件后替换；`MYTOOLS_QUICK_TEXT_DIR` 可覆盖数据目录。
文件包含短语及完整文本，重启后保留；数据损坏时报告错误，不自动覆盖。

## 进程查询

Windows 通过 netstat 获取本地 TCP/UDP 端口的进程，macOS 通过系统 lsof 获取。
显示当前用户可见的信息；结束权限由操作系统决定。终止前校验启动时间，避免 PID 被复用后误杀。

## 脚本管理（Windows 执行）

1. 在启动器搜索 **脚本** 或输入 `scripts `，选中 **管理脚本** 并按 Enter。
2. 在独立窗口点击 **新建脚本**，录入名称、可选搜索别名、多行脚本内容，选择 **PowerShell (.ps1)** 或 **CMD (.cmd)**，然后保存。可填写工作目录，留空使用用户主目录。
3. 回到启动器，输入名称或别名找到脚本。按 Enter **普通执行**；通过 **操作**（`Ctrl+K`）选 **管理员执行**，由 Windows 显示 UAC 确认。管理窗口也有这两个按钮。
4. **运行方式**决定是否显示终端：**窗口**打开终端并在结束后保留；**成功后关闭**在退出码为 0 时自动关闭终端，出错时暂停保留输出；**后台**不显示窗口、不接收输入（`Read-Host`、`pause` 等会直接失败或跳过），只在失败时弹窗显示退出码和最后几行输出，完整输出写在该次运行目录的 `output.log`。后台运行无法从界面中途停止，长时间运行的脚本只能在任务管理器结束。
5. 脚本数据保存在 `%LOCALAPPDATA%\MyTools\scripts\scripts.json`，可以用 `MYTOOLS_SCRIPTS_DIR` 指定数据目录。每次执行使用单独的脚本文件，后续编辑不会改动正在运行的脚本。

macOS 可以管理脚本条目，暂不支持运行 Windows PowerShell/CMD 脚本。

## 设置

在启动器输入 `settings`、`设置` 或别名 `settings `，选中 **打开设置** 打开设置窗口，左侧有 **插件** 和 **日志** 两页。**插件** 页列出全部已安装插件的名称、ID、运行状态、启动方式、别名、快捷键和启用状态：

- 名称旁的圆点显示插件当前状态：绿色运行中，黄色启动中，红色已停止，实心灰色已停用，空心灰色为按需启动、尚未用到。鼠标悬停显示状态和原因（例如“已停止：10 秒内未完成启动，已暂时停用”）。窗口每半秒刷新一次状态。
- 已停止的插件旁有 **重启** 按钮，点击后 MyTools 立即重新启动该插件。
- ID 旁的按钮切换启动方式：“随 MyTools 启动”的插件在 MyTools 启动时加载；“按需启动”的插件在第一次通过别名、快捷键或动作用到时才加载。默认由插件自己决定（参与全局搜索的插件和剪贴板历史随 MyTools 启动，其余按需启动）。
- 别名可以写多个，用空格或逗号分隔；输入框留空表示不使用别名，右侧按钮恢复插件自带的默认别名。
- 搜索插件的卡片下方列出它的快捷键：第一行“打开启动器并进入插件”，其后是插件声明的每个命令（如剪贴板历史的“以纯文本粘贴剪贴板”）。按 `Ctrl+Shift+V` 的格式填写，留空表示不使用，输入框为空时以浅色显示默认值，右侧按钮恢复默认。快捷键不能重复，也不能使用 `Alt+Space`；保存时校验。
- 关闭开关会停用插件：不再启动其进程，也不参与搜索和快捷键；通过别名搜索已停用的插件时会提示它已停用。设置插件本身不能停用。
- 已启用插件之间的别名不能重复，别名不能包含空白；保存时校验，失败时显示原因。
- 保存后写入 MyTools 目录下的 `plugin-settings.json`（只记录与默认不同的项），MyTools 随即重新加载插件并重新注册快捷键，无需重启。已被其他程序占用的快捷键在右下角提示。

**日志** 页显示 MyTools 和各插件最近的 2000 条日志，最新的在最上面：

- 可以按来源（全部、MyTools 或某个插件）、按级别（全部级别、信息及以上、警告及以上、仅错误）筛选，并在搜索框中按关键词查找（匹配内容、事件代码、来源和详情）。默认显示信息及以上。
- 打开期间每半秒读取一次新增的日志，日志文件轮转后会自动重新读取。点 **暂停** 停止刷新，方便查看；点 **继续** 恢复。
- 点击一条日志展开完整内容、事件代码、精确到毫秒的时间和详情（如崩溃的调用栈），可以复制这一条；**复制全部** 复制当前筛选出的所有日志。
- **打开文件夹** 打开日志所在的 `logs` 目录。

## 崩溃测试

`crash-test` 是一个故意出问题的插件（`global: true`，别名 `crash`），用来检验 MyTools 对插件异常的处理：状态圆点、右下角提示、自动重启和崩溃日志。输入 `crash` 可以看到它当前的进程号和下次崩溃的倒计时；别名后再输入内容会多返回一些测试结果：数字 1–50 表示结果条数（如 `crash 5`），其他内容返回 3 条回显该内容的结果，按 Enter 复制；全局搜索中只有搜索词包含 `crash` 或“崩溃”时才会出现。

各种异常在插件目录（部署后是 `C:\soft\MyTools\plugins\crash-test`）的 `config.json` 中分别配置，参考同目录的 `config.example.json`；没有这个文件时使用默认值，即不模拟任何异常；要开始测试，至少写入 `{"enabled": true}`（此时默认只有运行期间每 30 秒崩溃一次）。修改后在设置里重启插件，或等它下次崩溃重启后生效。

| 字段 | 默认 | 模拟的异常 | MyTools 的表现 |
| --- | --- | --- | --- |
| `enabled` | `false` | 总开关：`false` 时插件正常运行，不模拟任何异常；设为 `true` 后下面各项才生效 | |
| `startupCrash` | `false` | 每次启动都在初始化中崩溃 | 红点“启动过程中退出”，右下角提示；在设置中重启会再次崩溃，改回 `false` 后重启即可恢复 |
| `startupDelayMs` | `0` | 初始化耗时，每秒报告一次进度 | 黄点并显示进度；超过启动时限（默认 10 秒）则暂停并提示 |
| `startupError` | `""` | 非空时初始化失败，原因为该文字 | 红点“初始化失败：…”，右下角提示 |
| `runtimeCrash` | `true` | 就绪后每隔 `crashIntervalSecs` 秒崩溃一次 | 自动重启；60 秒内超过 5 次则停止并提示 |
| `crashIntervalSecs` | `30` | 运行期间崩溃的间隔 | |
| `pongDelayMs` | `0` | 心跳回复延迟的毫秒数 | 超过 3 秒没有回复即重启插件 |
| `pongDelayIntervalSecs` | `0` | 两次延迟回复之间至少间隔的秒数；`0` 表示每次都延迟 | |
| `searchDelayMs` | `0` | 每次搜索的耗时 | 超过等待时间时显示超时；同一次搜索执行超过 20 秒时重启插件 |

设置环境变量 `MYTOOLS_CRASH_TEST=off` 可无视配置文件关闭全部异常；也可以直接在设置中停用这个插件。两种崩溃都以 panic 结束进程，会记录到 `logs/crash.log`。

## 保持唤醒

原理与 PowerToys Awake 的“无限期 + 保持屏幕开启”相同：插件随 MyTools 启动，在主线程上调用
Windows 的 `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)`，
macOS 上创建 `PreventUserIdleSystemSleep` 和 `PreventUserIdleDisplaySleep` 两个电源断言（与 `caffeinate -di` 相同）。

- 不修改电源计划。插件被停用、MyTools 退出或插件崩溃时，系统自动撤销请求，恢复原有的睡眠和关屏设置。
- 只阻止空闲导致的睡眠和关屏；手动锁屏、合上笔记本盖子、按电源键仍按系统设置执行，公司组策略强制的锁屏也可能仍会生效。
- 用 `powercfg /requests`（Windows，管理员）或 `pmset -g assertions`（macOS）可以看到 MyTools 的请求。
