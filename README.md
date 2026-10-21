# CrazyRenamer

Windows 资源管理器批量更名工具，支持多种改名规则、实时预览与一键应用。

## 功能特性

- **多种更名规则**
  - 替换：查找内容 / 替换为，支持保留扩展名、忽略大小写
  - 正则：Rust regex 语法，支持 `$1` 捕获组
  - 编号：模板 `{name}_{n}`（`{name}` 为原名称、`{n}` 为编号），可设起始、步长、位数
  - 日期：格式 `yyyy-mm-dd` 或 `yyyymmdd`，添加到名称前缀或后缀
  - 前后缀：前缀 / 后缀可同时使用
- **载入文件夹**
  - 在资源管理器文件夹内**点击鼠标中键**直接载入当前文件夹（可在界面中开关）
  - 也支持手动输入路径、浏览选择、刷新
- **范围与筛选**
  - 可选文件 / 文件夹（目录批量重命名），文件夹仅处理第一层
  - 扩展名多选过滤（含“无扩展名”），仅处理勾选的扩展名
- **更名预览**
  - 载入后即时显示 原名称 → 新名称 与状态（将更名 / 名称不变 / 问题）
  - 自动检测名称冲突、非法名称等，有问题时禁止应用
- **系统托盘**
  - 最小化 / 关闭窗口时隐藏到托盘，程序驻留后台
  - 右键托盘图标可退出

## 使用说明

1. 启动程序，在资源管理器中打开目标文件夹后点击鼠标中键（或手动载入路径）。
2. 在“范围与筛选”中选择处理文件 / 文件夹，并勾选需要处理的扩展名。
3. 在“更名规则”中选择规则模式并填写参数，下方列表实时预览结果。
4. 确认无误后点击底部“应用更名”执行。

## 构建

环境要求：

- Windows 10 / 11（x64）
- Rust 稳定版（edition 2024）
- MinGW-w64（`windres`，用于编译图标资源）

```bash
cargo build --release
```

产物为 `target/release/CrazyRenamer.exe`，双击即可运行，无需安装。

> 项目内 `.cargo/config.toml` 已配置 USTC crates.io 镜像（可按需保留或删除）。

## 测试

```bash
cargo test
```

## 项目结构

```
src/
├── main.rs      # 界面（egui）与应用逻辑
├── rename.rs    # 更名规则、预览计划与冲突校验
├── explorer.rs  # 资源管理器窗口 / 当前文件夹定位
├── hook.rs      # 鼠标中键全局钩子
└── tray.rs      # 系统托盘
assets/          # 图标资源（ico / rc / png）
build.rs         # 将图标嵌入可执行文件
```

## 技术栈

- [Rust](https://www.rust-lang.org/)（edition 2024）
- [eframe / egui 0.37](https://github.com/emilk/egui)（glow 渲染）
- [regex](https://github.com/rust-lang/regex)、[rfd](https://github.com/PolyMeilex/rfd)（文件选择对话框）
- [windows-rs](https://github.com/microsoft/windows-rs)（窗口、托盘与钩子相关 Win32 API）
