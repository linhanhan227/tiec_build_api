# TieCloud Build API

TieCloud 的构建服务后端，提供 .tsp（ZIP）项目上传、构建任务创建、状态/事件查询与 APK 下载能力，并内置 OpenAPI/Swagger 文档与多级限流。

## 功能概览

- 上传 .tsp（ZIP）项目文件并生成 `file_id`（SHA1）
- 创建构建任务并返回 `task_id`
- 查询任务状态与构建事件（带分页）
- 构建成功后下载 APK
- 健康检查与运行统计
- 内置 OpenAPI/Swagger 文档

## 接口与返回格式

JSON 接口统一返回结构：

```json
{ "code": 200, "data": { ... } }
```

接口一览：

- GET /health
- POST /api/v1/upload（multipart/form-data，字段名为 `file`）
- POST /api/v1/build
- GET /api/v1/build/{id}/status
- GET /api/v1/build/{id}/events?limit&offset
- GET /api/v1/build/{id}/download（文件下载，非 JSON）

## 任务状态

任务状态值（中文枚举）：

- 排队中
- 处理中
- 编译成功
- 编译失败
- 编译超时
- 已取消
- 未知错误

## 构建流程（简版）

1) 上传：校验扩展名与 MIME、检查 ZIP 完整性，最大 100MB，生成 `file_id`（SHA1）。
2) 创建任务：按 `file_id + 用户 IP` 去重，写入 SQLite 并入队。
3) Worker 构建：解压项目、准备资源（.tiec/stdlib、.tiec/tiecc），调用编译器与 Gradle 生成 APK。
4) 产物：查找 APK 并写入 `output_path`，可通过下载接口获取。

## 快速开始

1) 构建

```bash
cargo build --release
```

2) 运行（默认监听 0.0.0.0:8080）

```bash
./target/release/tie_api_server
```

如需自定义端口或路径，可通过环境变量启动：

```bash
HOST=127.0.0.1 PORT=8081 UPLOAD_DIR=./.tiec/uploads DATABASE_PATH=./.tiec/tasks.db \
./target/release/tie_api_server
```

3) 查看 Swagger 文档

```text
http://localhost:8080/swagger-ui/
```

## build-test 自测模式

使用 `--build-test` 启动后，会按真实 API 流程自动执行上传与构建：

```bash
./target/release/tie_api_server --build-test
```

配置来源：

- `test.json`（默认在当前目录，亦可通过 `BUILD_TEST_CONFIG` 指定路径）
- 环境变量：`BUILD_TEST_BASE_URL`、`BUILD_TEST_INTERVAL_MS`、`BUILD_TEST_FILE_PATH`、`BUILD_TEST_FILE_PATHS`

`test.json` 可包含：

```json
{
	"base_url": "http://127.0.0.1:8080",
	"file_path": "./demo.tsp",
	"file_paths": ["./demo1.tsp", "./demo2.tsp"],
	"interval_ms": 200,
	"task_timeout": 900,
	"queue_capacity": 30,
	"cleanup_interval": 30,
	"cleanup_retention_secs": 15,
	"hourly_ip_limit": 60,
	"max_retries": 3
}
```

## 配置

通过环境变量覆盖默认值（未设置则使用默认值）：

| 变量名 | 默认值 | 说明 |
| --- | --- | --- |
| HOST | 0.0.0.0 | 监听地址 |
| PORT | 8080 | 监听端口 |
| UPLOAD_DIR | ./.tiec/uploads | 上传与解压目录（建议使用绝对路径） |
| DATABASE_PATH | ./.tiec/tasks.db | SQLite 数据库路径 |
| QUEUE_CAPACITY | 15 | 队列容量 |
| WORKER_COUNT | 1 | Worker 数量 |
| TASK_TIMEOUT | 900 | 单任务超时（秒） |
| CLEANUP_INTERVAL | 3600 | 清理任务间隔（秒） |
| CLEANUP_RETENTION_SECS | 3600 | 清理保留窗口（秒） |
| HOURLY_IP_LIMIT | 20 | 每 IP 每小时限制（仅对上传/构建相关接口生效） |
| MAX_RETRIES | 3 | 失败重试次数上限 |

## 运行时目录/文件

- 上传目录：`UPLOAD_DIR`（默认 ./.tiec/uploads）
- 解压目录：`{UPLOAD_DIR}/{file_id}`
- 内置资源：`.tiec/stdlib`、`.tiec/tiecc`（启动时自动解压）
- Android 基础库：`.tiec/安卓基本库`（启动时初始化）
- SQLite 数据库：`DATABASE_PATH`（默认 ./.tiec/tasks.db）
- IP 封禁列表：`.tiec/ip_ban.txt`

## 速率限制说明

- 秒级限流（双层）：
	- Governor：每 IP 每秒 120 次（突发 120）。
	- 自定义 IP 限流：超限将封禁 7 天，并写入 `.tiec/ip_ban.txt`。
- 小时级限流：对上传与构建相关接口生效（上传、构建、状态、事件、下载），默认每 IP 每小时 20 次。

## 目录结构（核心）

- src/api：API 处理器
- src/worker：任务执行与清理
- src/state：内存状态与队列
- src/database：SQLite 访问与迁移
- src/middleware：限流与封禁
- src/models：任务与 API 模型

## 文档

- [API文档.md](API文档.md)
- [项目完整文档.md](项目完整文档.md)
- [代码深度解析.md](代码深度解析.md)

同时提供 OpenAPI JSON：

- /api-docs/openapi.json
- /swagger-ui/

## License

内部使用或按项目要求指定。
