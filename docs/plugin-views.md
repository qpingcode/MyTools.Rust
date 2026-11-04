# 插件视图与动作约定

宿主提供布局，插件提供数据。当前支持普通列表 `list` 和左列表右预览 `list-detail`。
布局、主题、键盘导航由 Slint 组件实现；插件进程不注入 Slint 源码、HTML 或本机控件。
现有 `type: window` 仍可启动完全独立的插件窗口。后续新增表格、网格等布局时扩展视图枚举，
不改变插件进程隔离和请求响应方式。

## 清单

```json
{
  "id": "clipboard",
  "name": { "key": "name", "defaultValue": "Clipboard History" },
  "version": "0.1.0",
  "type": "search",
  "entry": "mytools-clipboard",
  "icon": "builtin:clipboard",
  "alias": ["clip"],
  "search": { "global": false },
  "view": "list-detail",
  "hotkey": "Ctrl+Shift+V"
}
```

`hotkey` 进入第一个别名的作用域；宿主配置 `plugin_shortcuts.<id>` 覆盖它，空值禁用。
唤起时搜索框填入别名和末尾空格，并将光标移到末尾，直接输入即可追加查询内容。
快捷键冲突时显示错误，其余注册成功的快捷键继续工作。搜索插件仍在启动时加载，
`global: false` 只控制是否参与普通搜索。移除该插件无需修改宿主代码。

## 搜索与预览

请求沿用 `{"query":"关键词"}`，响应例如：

```json
{
  "revision": 42,
  "items": [{
    "title": "记录标题",
    "description": "时间及大小",
    "icon": "builtin:file-text",
    "resource": { "type": "plugin_item", "id": "opaque-stable-id" },
    "preview": {
      "title": "记录标题",
      "text": "简短预览",
      "metadata": "时间及大小",
      "text_file": "C:\\plugin-data\\full-text.txt"
    },
    "actions": [
      { "id": "plugin", "command": "paste", "title": "粘贴到原应用" },
      { "id": "plugin", "command": "delete", "title": "删除记录" }
    ]
  }]
}
```

`preview` 也适用于普通资源；`plugin_item.id` 是宿主不解释的稳定标识。
`preview.image` 为本地缩略图的绝对路径，等比显示、保留颜色；
`preview.text_file` 是完整 UTF-8 文本的绝对路径（宿主读取上限 2 MiB），缺失时使用 `text`。
图片/文本在选择记录时读取，不把完整图片或大段文本嵌入 JSON；JSON 行仍限 1 MiB、每次最多 50 项。
大图原始内容由插件自行存储和恢复。文字按纯文本呈现，不执行 HTML。

列表和详情视图都不定时查询。宿主在输入变化时发起搜索，收到插件响应后展示结果；
自定义动作完成并返回 `effect.refresh: true` 时再查询一次。
剪贴板历史展示的是查询时的快照，不在展示期间动态更新。
插件不得主动向 stdout 推送通知，避免破坏有序传输。
已有插件可省略 `view`、`revision` 和 `preview`；`revision` 仍兼容读取，但不用于驱动查询。

## 自定义动作

宿主将动作投递回产生该条结果的同一常驻进程：

```json
{"query":"","action":{"command":"paste","item":"opaque-stable-id"}}
```

成功响应：

```json
{"items":[],"effect":{"refresh":false,"paste":true,"message":"已复制到剪贴板"}}
```

插件先完成自身操作再回复；`refresh` 请求刷新当前作用域，`paste` 请求宿主返回原应用并粘贴。
`effect.copy_text` 可携带待复制文本，由宿主在 UI 线程写入剪贴板；可和 `paste` 组合使用。
剪贴板插件负责恢复内容，宿主负责焦点、快捷键释放等待及平台输入适配。
返回 `{"items":[],"error":"操作失败的本地化说明"}` 表示业务失败，不停止插件进程。
自定义命令按 `(id, command)` 去重，保留插件声明的顺序；`title` 由插件翻译，宿主原样显示。
动作面板继续绑定打开时的结果快照，避免列表更新后误操作另一条记录。
首个动作显示在底栏并通过 Enter 执行；Actions 菜单仅展示后续动作。没有后续动作时按钮置灰，快捷键不打开空菜单。

Rust 插件使用 `serve_interactive(search, action)`，仅搜索插件继续使用原有 `serve(search)`。
所有请求/响应各占一行并刷新输出；stdout 专用于协议，诊断写入 stderr。
每个请求按顺序对应一行响应；心跳 `{"ping":n}` 例外，插件必须立即回复 `{"pong":n,"busyMs":毫秒}`，可插在其他响应之间，
即使正在处理请求也要回复（`plugin-api` 的读线程已实现，Rust 插件无需额外代码）。不经过 `plugin-api` 的插件需自行实现。
搜索请求带 `timeoutMs`，表示宿主本次最多等待多久：全局搜索 1 秒，别名搜索使用 `search.aliasTimeoutMs`（默认 3 秒，最多 15 秒）。
宿主停止等待的搜索不会终止进程；收到更新的搜索时，`plugin-api` 让排队中的旧搜索直接返回空响应，执行中的搜索可通过
`cancelled()` 提前结束。动作等待上限 60 秒，允许网络动作完成后再回复。
动作由宿主工作线程等待，不阻塞 UI。动作进行期间不向同一插件排入搜索，避免搜索排在长动作后面；
其他插件仍可查询。动作超时会停止该进程，避免后续响应错配。

## 文本工具交互

encode、decode 使用 `list`，各格式各占一行：主标题显示转换结果，副标题显示格式，Enter 复制完整结果。formatter、translator 使用
`list-detail`，用已有详情区展示多行结果；无需为只读文本转换新增视图枚举。
输入可使用 `@绝对文件路径` 加载 UTF-8 文本（最多 16 KiB）；文件路径含空格时也可加双引号。
直接输入的内部空白保持原样；以 `@` 开头的字面文本请通过文件输入。

翻译搜索仅准备请求，显式执行“翻译”动作才联网，请求最长 45 秒。
插件等待 API 完成，保存结果后返回 `effect.refresh: true`，宿主再查询一次并展示译文或失败说明。
相同输入的结果在进程内缓存，最多 32 条，不写入磁盘。失败可重新执行翻译，无轮询或自动重试。

快捷文本使用 `qt add <短语> | <文本或 @文件路径>` 准备待保存项，Enter 才写入；
相同短语覆盖更新。保存项返回 `plugin_item`，由插件复制或删除；复制使用 `effect.copy_text`。
进程管理也使用 `plugin_item`，动作中的 PID 与进程启动时间绑定，执行前重新核实身份。

## 参考实现

- `plugins/clipboard/src/clipboard/`：平台监听与剪贴板格式恢复。
- `plugins/clipboard/src/store.rs`：数据、去重和保留策略。
- `plugins/clipboard/src/presentation.rs`：结果、预览和动作声明。
- `crates/launcher/ui/components/detail-preview.slint`：共享详情布局。
- `crates/launcher/src/desktop/`：焦点恢复与跨应用粘贴。
