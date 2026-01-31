# TieCloud Build API

TieCloud Build API 是一个基于 Rust 和 Actix Web 开发的高性能 REST API 服务，专为 Tie 语言项目的云端构建设计。它提供了一套完整的解决方案，用于管理文件上传、异步构建任务调度、实时状态追踪以及构建产物分发。

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)
![Actix](https://img.shields.io/badge/framework-ActixWeb-green.svg)

## 📖 核心特性

- **高性能架构**: 基于 Actix Web 4.x 和 Tokio 异步运行时，支持高并发请求处理。
- **异步任务队列**: 基于数据库租约队列的异步调度，支持多 Worker 并发与重启恢复。
- **数据持久化**: 集成 SQLite 数据库，全生命周期记录任务 ID、状态、进度及错误信息，服务重启不丢失数据。
- **智能编译器管理**: 采用**运行时动态加载**策略，根据宿主机操作系统（Linux/macOS/Windows）自动适配对应的 `tiec` 编译器二进制。
- **自动化运维**: 内置定时清理任务（Cron-like），自动回收过期临时文件与构建产物，防止磁盘空间耗尽。
- **OpenAPI 支持**: 集成 Swagger UI (utoipa)，提供开箱即用的交互式 API 文档。

## 🏗 系统架构

TieCloud Build API 采用清晰的分层架构设计，确保系统的可维护性与扩展性。

### 架构分层

1.  **Web 接入层 (API Layer)**
    - 处理 HTTP 请求（Upload, Build, Query, Download）。
    - 负责参数校验、鉴权（预留）和统一响应封装。
    - 位于 `src/api/`。

2.  **任务调度层 (Scheduling Layer)**
    - 基于数据库租约队列进行任务拉取与续租，避免重复消费。
    - Worker 并发执行构建任务，确保有限的构建进程同时运行，避免耗尽服务器资源。
    - 位于 `src/worker/`。

3.  **构建执行层 (Execution Layer)**
    - 负责与底层文件系统交互，管理 `.tsp` 项目文件的解压与资源注入。
    - 调用外部 `tiec` 进程执行实际编译工作。
    - 实时捕获标准输出/错误流（stdout/stderr），解析构建进度。

4.  **数据持久层 (Persistence Layer)**
    - 使用 SQLite 存储任务元数据。
    - 位于 `src/database.rs`。

### 目录结构说明

```text
.
├── Cargo.toml          # 项目依赖配置
├── build.rs            # 编译脚本：负责将静态资源和编译器复制到输出目录
├── README.md           # 项目文档
├── src/
│   ├── main.rs         # 程序入口
│   ├── config.rs       # 配置管理
│   ├── database.rs     # SQLite 数据库操作封装
│   ├── api/            # API 路由定义与处理函数
│   ├── worker/         # 任务队列与构建逻辑核心
│   ├── models/         # 数据结构 (DTO/DAO)
│   └── stdlib/         # Tie 语言标准库 (Android/Web/Windows)
└── tiecc/              # 编译器二进制库 (构建时自动分发)
    ├── linux_x86_64/
    ├── linux_arm64-v8a/
    ├── macos/
    └── win_x86_64/
```

## 🚀 快速开始

### 1. 环境要求

- **Rust**: 1.70 或更高版本
- **Cargo**: Rust 包管理器
- **SQLite3**: 运行时依赖库
- **操作系统**: macOS, Linux (x86_64/arm64), 或 Windows

### 2. 构建命令

本项目利用 `build.rs` 在编译阶段处理资源依赖。

```bash
# 1. 克隆项目
git clone <repository_url>
cd TieApi

# 2. 检查编译器资源
# 确保项目根目录下的 tiecc/ 文件夹中包含对应平台的编译器二进制文件

# 3. 编译发布版本
cargo build --release
```

构建成功后，可执行文件位于 `target/release/tie_api_server`。

### 3. 运行服务

直接运行编译后的二进制文件。服务启动时会自动初始化 SQLite 数据库和工作目录。

```bash
# 运行服务
./target/release/tie_api_server
```

启动日志示例：
```text
INFO  tie_api_server > Starting TieCloud Build API
INFO  tie_api_server > Environment: Production
INFO  tie_api_server > Listening on http://0.0.0.0:8080
INFO  tie_api_server > Database initialized at ./.tiec/tasks.db
```

访问地址：
- **API Base URL**: `http://localhost:8080/api/v1`
- **Swagger UI**: `http://localhost:8080/swagger-ui/`

## ⚙️ 配置说明

所有配置均通过环境变量管理，支持 `.env` 文件。

| 变量名 | 默认值 | 描述 |
|--------|--------|------|
| `HOST` | `0.0.0.0` | 服务器监听地址 |
| `PORT` | `8080` | 服务器监听端口 |
| `UPLOAD_DIR` | `./.tiec/uploads` | 上传文件临时存储路径 |
| `DATABASE_PATH` | `./.tiec/tasks.db` | SQLite 数据库文件路径 |
| `WORKER_COUNT` | `1` | 并发构建工作线程数（建议设置为 CPU 核心数 / 2） |
| `QUEUE_CAPACITY` | `15` | 等待队列最大容量 |
| `TASK_TIMEOUT` | `900` | 单个任务超时时间（秒，默认15分钟） |
| `HOURLY_IP_LIMIT` | `20` | 上传/构建相关接口每小时每IP最大请求数 |
| `RUST_LOG` | `info` | 日志级别 (debug, info, warn, error) |

## 📦 部署指南

### 单文件部署

由于采用了静态资源内嵌或运行时复制策略，部署非常简单：

1.  将 `target/release/tie_api_server` 上传至服务器。
2.  确保二进制文件同级目录下存在 `tiecc/` 目录（构建系统会自动复制，部署时请一并将该目录上传）。
3.  运行即可。

**目录结构示例 (部署后)**:
```text
/opt/tie-cloud/
├── tie_api_server          # 主程序
├── tiecc/                  # 编译器目录 (必须)
│   ├── linux_x86_64/tiec
│   └── ...
└── .tiec/                  # 运行时自动生成的目录 (数据、日志、临时文件)
```

### 编译器更新

如需更新底层的 `tiec` 编译器，**无需重新编译** API 服务：

1.  停止 API 服务。
2.  替换 `tiecc/` 目录下对应平台的 `tiec` 二进制文件。
3.  重启 API 服务。

## 🔌 API 接口概览

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/health` | 服务健康检查 |
| `POST` | `/api/v1/upload` | 上传 `.tsp` 项目包 |
| `POST` | `/api/v1/build` | 提交构建任务 |
| `GET` | `/api/v1/build/{id}/status` | 查询任务状态与进度 |
| `GET` | `/api/v1/build/{id}/events` | 查询任务事件审计（支持分页） |
| `GET` | `/api/v1/build/{id}/download` | 下载构建成功的 APK |
| `POST` | `/api/v1/admin/cleanup` | (管理) 手动触发垃圾清理 |

**接口限流说明**：上传与构建相关接口同一IP每小时最多 `HOURLY_IP_LIMIT` 次，超过将返回 429。

详细接口定义与参数说明，请参考启动后的 Swagger 文档。

---
&copy; 2026 TieCloud Team.
