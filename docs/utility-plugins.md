# 实用插件

运行 `./scripts/debug-run.ps1 -BuildOnly` 或 `bash scripts/debug-run.sh --build-only` 编译并打包全部插件。
别名后输入空格进入插件作用域。默认 Enter 执行首个动作，Ctrl+K（macOS 为 ⌘K）打开其他动作。

| 插件 | 输入示例 | 交互 |
| --- | --- | --- |
| Chrome 书签 | 直接输入书签标题、URL、文件夹关键词 | 全局搜索，不设置 alias；Enter 在系统默认浏览器打开，操作菜单可复制链接 |
| encode | `encode hello`、`encode base64 hello` | 普通列表展示各格式结果；Enter 复制 |
| decode | `decode base64 5Lit5paH` | 普通列表展示各格式结果；Enter 复制 |
| formatter | `fmt json {"a":1}`、`fmt xml @C:\Temp\input.xml` | 详情展示格式化结果；Enter 复制 |
| process-killer | `ps chrome`、`ps :8080`、`ps port 8080`、`ps 8080` | 按进程名称或本地 TCP/UDP 端口查询；Enter 或“结束进程”动作终止所选进程 |
| UUID generator | `uuid `、`uuid v7`、`uuid v4 10` | UUID v4/v7；单个提供大小写、无连字符、花括号格式，批量提供复制全部；最多 50 个 |
| translator | `tr hello`、`tr en 你好`、`tr zh Hello`、`tr` | Enter 发起翻译，完成后 Enter 复制译文；单词另有音标（Enter 朗读）、英文释义和例句；只输入 `tr` 列出最近翻译 |
| quick text | `qt add email | hello@example.com`，然后 `email` 或 `qt email` | Enter 保存；全局按短语、内容查找；Enter 复制，“删除快捷文本”动作删除 |

encode 按 `base64`、`md5`、`url`、`sha1`、`sha256`、`sha512` 的顺序展示结果。
decode 按 `base64`、`url` 的顺序展示结果。URL 使用 URI component 编码，`+` 保持加号；
哈希不能解码。格式名使用这里的小写值。
decode 严格验证编码与 UTF-8，二进制结果显示错误，不把无效字节替换成乱码。

formatter 仅支持 `json`、`xml`。JSON 保留对象键顺序及数字精度。
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
| `cache_size` | `100` | 本地保存的最近翻译条数，`0` 关闭缓存和历史 |
| `speech.voice` | `"auto"` | `"auto"` 自动选美式英语语音，或填写语音名称，如 `"Microsoft Zira Desktop"`、macOS 的 `"Samantha"` |
| `speech.rate` | `0` | 语速，`-10`（最慢）到 `10`（最快） |

默认模型与请求格式参考 [DeepSeek 官方文档](https://api-docs.deepseek.com/quick_start)。
配置及环境变量在每次执行翻译时读取；macOS 修改环境变量后需要从带有新环境的进程重新启动 MyTools。
不带方向时，含汉字的文本译成英文，其他文本译成简体中文；可显式使用 `en` 或 `zh`。
输入完成后按 Enter 才发送文本；插件等 API 完成后回复，宿主收到回复后再查询一次读取结果。
同一时间只执行一个请求，全程不轮询。
超时、鉴权失败、限流及响应截断会显示“翻译失败，按 Enter 重试”，副标题和详情面板显示完整错误原因（含所用代理）。

### 缓存与最近记录

翻译成功后保存到 `%LOCALAPPDATA%\MyTools	ranslator\cache.json`（macOS 为
`~/Library/Application Support/MyTools/translator/cache.json`，可用 `MYTOOLS_TRANSLATOR_CACHE` 指定），
保留最近 `cache_size` 条，满了删除最早翻译的一条。再次输入相同内容时直接显示译文，不再请求 API；
`Ctrl+K` →“重新翻译”会重新请求并覆盖缓存。失败结果只保存在内存中。只输入 `tr` 时按时间倒序列出最近的翻译。

### 单词：音标、朗读、释义与例句

输入单个英文单词（如 `tr hello`），或输入简短中文且译文是单个英文单词（如 `tr 苹果` → apple）时，
同一次请求额外返回该英文单词的信息，依次显示：

1. 译文 — Enter 复制；`Ctrl+K` 可朗读或重新翻译
2. 音标，如 `/həˈləʊ/` — Enter 用系统语音朗读（Windows SAPI / macOS `say`，离线）
3. 英文释义 — Enter 复制
4. 两三条例句及中文翻译 — Enter 复制例句，`Ctrl+K` 可朗读

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

在启动器输入 `settings`、`设置` 或别名 `settings `，选中 **打开设置** 打开设置窗口。**插件** 页列出全部已安装插件的名称、ID、运行状态、启动方式、别名和启用状态：

- 名称旁的圆点显示插件当前状态：绿色运行中，黄色启动中，红色已停止，实心灰色已停用，空心灰色为按需启动、尚未用到。鼠标悬停显示状态和原因（例如“已停止：10 秒内未完成启动，已暂时停用”）。窗口每半秒刷新一次状态。
- 已停止的插件旁有 **重启** 按钮，点击后 MyTools 立即重新启动该插件。
- ID 旁的按钮切换启动方式：“随 MyTools 启动”的插件在 MyTools 启动时加载；“按需启动”的插件在第一次通过别名、快捷键或动作用到时才加载。默认由插件自己决定（参与全局搜索的插件和剪贴板历史随 MyTools 启动，其余按需启动）。
- 别名可以写多个，用空格或逗号分隔；输入框留空表示不使用别名，右侧按钮恢复插件自带的默认别名。
- 关闭开关会停用插件：不再启动其进程，也不参与搜索和快捷键；通过别名搜索已停用的插件时会提示它已停用。设置插件本身不能停用。
- 已启用插件之间的别名不能重复，别名不能包含空白；保存时校验，失败时显示原因。
- 保存后写入 MyTools 目录下的 `plugin-settings.json`（只记录与默认不同的项），MyTools 随即重新加载插件，无需重启。插件快捷键的启用状态在重启 MyTools 后生效。

## 崩溃测试

`crash-test` 是一个故意出问题的插件（`global: true`，别名 `crash`），用来检验 MyTools 对插件异常的处理：状态圆点、右下角提示、自动重启和崩溃日志。输入 `crash`（或 `crash` 加空格后接任意内容）可以看到它当前的进程号和下次崩溃的倒计时；全局搜索中只有搜索词包含 `crash` 或“崩溃”时才会出现。

各种异常在插件目录（部署后是 `C:\soft\MyTools\plugins\crash-test`）的 `config.json` 中分别配置，参考同目录的 `config.example.json`；没有这个文件时使用默认值。修改后在设置里重启插件，或等它下次崩溃重启后生效。

| 字段 | 默认 | 模拟的异常 | MyTools 的表现 |
| --- | --- | --- | --- |
| `enabled` | `true` | `false` 关闭全部异常，插件正常运行 | |
| `startupCrash` | `true` | 第一次启动时在初始化中崩溃，只发生一次（插件目录下的 `startup-crashed` 记录已发生，删除它可再次触发） | 红点“启动过程中退出”，右下角提示，需在设置中重启 |
| `startupDelayMs` | `0` | 初始化耗时，每秒报告一次进度 | 黄点并显示进度；超过启动时限（默认 10 秒）则暂停并提示 |
| `startupError` | `""` | 非空时初始化失败，原因为该文字 | 红点“初始化失败：…”，右下角提示 |
| `runtimeCrash` | `true` | 就绪后每隔 `crashIntervalSecs` 秒崩溃一次 | 自动重启；60 秒内超过 5 次则停止并提示 |
| `crashIntervalSecs` | `30` | 运行期间崩溃的间隔 | |
| `pongDelayMs` | `0` | 心跳回复延迟的毫秒数 | 超过 3 秒没有回复即重启插件 |
| `pongDelayIntervalSecs` | `0` | 两次延迟回复之间至少间隔的秒数；`0` 表示每次都延迟 | |
| `searchDelayMs` | `0` | 每次搜索的耗时 | 超过等待时间时显示超时；同一次搜索执行超过 20 秒时重启插件 |

设置环境变量 `MYTOOLS_CRASH_TEST=off` 可无视配置文件关闭全部异常；也可以直接在设置中停用这个插件。两种崩溃都以 panic 结束进程，会记录到 `logs/crash.log`。
