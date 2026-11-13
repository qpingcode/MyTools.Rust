# 项目约定

## Rust 文件与函数组织

- 保持实现简单，优先按职责组织模块，行数作为可读性参考，不作为硬性限制。
- 单个 Rust 源文件以 100～300 行为日常目标；300～500 行且职责统一时可以保留。
- 超过 500 行时，检查是否混合了多个职责，优先按明确职责拆分；超过 1000 行时通常应拆分。
- 上述行数建议主要衡量业务代码，不计文件底部的单元测试；测试规模单独评估，不因测试使文件总行数超过参考值而拆分业务模块。
- 不为满足行数目标压缩格式、机械拆文件，或引入不必要的抽象；生成代码不适用这些行数建议。
- 使用 Rust + Slint。界面布局、样式和交互组件放在 `.slint` 文件；Rust 负责状态协调和业务逻辑。
- 搜索框、结果项、底栏等按职责拆成 Slint 组件；共享主题和基础组件放在根目录 `ui/`，应用或插件专用组件放在各自的 `ui/`。
- `crates/core` 不依赖 UI 框架；平台差异隔离在平台模块中。避免为每个小功能新增 crate。
- 普通逻辑函数保持简短；复杂流程拆成职责清晰、名称明确的小函数。

## 测试组织

- 单元测试默认放在被测源文件底部，使用 `#[cfg(test)] mod tests { ... }`，暂不独立拆分测试文件。
- 集成测试放在对应 crate 下与 `src/` 同级的 `tests/` 目录，通过公开接口验证模块协作。
- 不为方便测试而把私有实现改成公开接口。

## 多语言

- 用户可见文案放在所属模块的 `locales/en.json` 和 `locales/zh-CN.json`，使用稳定的语义 key；完整句子用命名参数格式化，不拼接翻译片段。
- Slint 通过共享的 `I18n.text` 回调读取字典；Rust 使用 `mytools-plugin-api::i18n` 提供的语言和字典工具。语言在启动时确定，修改配置后重启生效。
- 通用 Action 的默认标题由 launcher 根据 core 提供的标题 key 翻译；插件自定义标题由插件负责，宿主按原文显示。
- 插件维护自己的语言文件，通过 `MYTOOLS_LOCALE` 继承宿主语言。插件名称按 `name.key` 查找，缺失时回退英文及 `defaultValue`；文件名、用户输入和别名保持原样。

## 分支

- 除非用户特别说明，只在 `master` 上开发，不新建分支，也不使用 git worktree。
- push 前先用 `gh auth status` 查看当前活动的 GitHub 账号；不是 `qpingcode` 时先执行 `gh auth switch --user qpingcode` 再 push，push 完成后（无论成功与否）用 `gh auth switch --user <原账号>` 切换回原来的账号。

## 提交

- 每次修改完成（测试通过）后自动 `git commit`，提交信息简要说明改动；不需要用户再要求。
- 不自动 push，只有用户明确要求时才 push。
- 换行由 `.gitattributes` 统一：Windows 工作区为 CRLF，仓库中保存 LF（`.sh` 始终为 LF）。

## 完成任务后更新本机程序

- 每次完成任务、测试通过后，执行 `./scripts/deploy.ps1`（PowerShell 7），把改动编译并更新到本机实际使用的 `C:\soft\MyTools`。
- 脚本以 release 编译整个 workspace，只替换内容有变化的文件；用户数据在 `%LOCALAPPDATA%\MyTools`，脚本不会改动。运行中的程序会先改名为 `*.old` 再替换；本体有变化时重启 MyTools，只有插件变化时只结束对应插件，由 MyTools 自动拉起。
- 若按用户要求在独立分支或 worktree 上完成任务，先合并回 `master` 再在主工作区部署，避免用缺少其他改动的版本覆盖已部署的程序。
- 可选参数：`-Target <目录>` 指定安装目录，`-SkipBuild` 跳过编译，`-NoRestart` 不重启，等下次手动重启生效。
- 本机 VS 18 的 C++ 工具集不完整，脚本会自动加载工具集完整的 Visual Studio（目前是 VS 2019）的开发环境；部署失败时把编译错误原样报告给用户。
- 链接器使用 `rust-lld`（见 `.cargo/config.toml`），它依赖开发环境提供的库路径。手动执行 `cargo build/test/clippy` 前，先在同一个 PowerShell 里执行 `. ./scripts/dev-environment.ps1; Enter-DeveloperEnvironment`。始终使用同一套环境，避免依赖因环境变量变化而反复重编。
