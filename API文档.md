# TieCloud Build API 接口文档（完整）

本文档描述 TieCloud Build API 的所有公开接口、参数、响应与错误格式，内容与服务端实现保持一致。

## 1. 基础信息

- 协议：HTTP
- Base URL：`http://0.0.0.0:8080`
- API 前缀：`/api/v1`
- 响应编码：UTF-8
- 认证：当前版本无鉴权

## 2. 速率限制

服务内置两层限流：

1) **秒级限流**
- 每 IP 每秒 120 次
- 超限将被封禁 7 天（封禁信息写入 `.tiec/ip_ban.txt`）

2) **小时级限流**（仅对以下接口生效）
- 默认每 IP 每小时 20 次，可通过环境变量 `HOURLY_IP_LIMIT` 调整
- 生效接口：
	- POST /api/v1/upload
	- POST /api/v1/build
	- GET /api/v1/build/{id}/status
	- GET /api/v1/build/{id}/events
	- GET /api/v1/build/{id}/download

> 限流触发时，服务会返回 403 或 429，错误响应格式为 `{ error, message, ... }`，不一定符合通用错误响应格式。

## 3. 通用响应格式

### 3.1 成功响应

除文件下载接口外，成功响应统一包裹为：

```json
{
	"code": 200,
	"data": {}
}
```

字段说明：

- `code`：HTTP 状态码（与响应状态一致，如 200/202）
- `data`：业务数据对象

### 3.2 错误响应

所有错误均返回 JSON：

```json
{
	"code": 400,
	"message": "Bad Request"
}
```

字段说明：

- `code`：HTTP 状态码
- `message`：错误描述

## 4. 数据模型

以下模型为 `data` 字段内容。

### 4.1 UploadResponse

```json
{
	"file_id": "sha1_hex"
}
```

- `file_id`：上传文件的 SHA1（40 位十六进制字符串）

### 4.2 BuildRequest

```json
{
	"file_id": "sha1_hex"
}
```

### 4.3 BuildResponse

```json
{
	"task_id": "uuid",
	"status": "排队中"
}
```

### 4.4 Task

```json
{
	"task_id": "uuid",
	"status": "处理中",
	"progress": 0,
	"estimated_time_remaining": 120,
	"current_step": "编译中",
	"error": null,
	"created_at": "2026-02-01T00:00:00Z",
	"updated_at": "2026-02-01T00:00:30Z",
	"retry_count": 0,
	"max_retries": 3,
	"build_duration": 120,
	"priority": 0
}
```

- `status` 可选值：`排队中`、`处理中`、`成功`、`失败`、`已取消`
- `progress`：0-100

### 4.5 TaskEvent

```json
{
	"id": 1,
	"task_id": "uuid",
	"event_type": "log",
	"status": "处理中",
	"message": "step message",
	"worker_id": "worker-0",
	"created_at": "2026-02-01T00:00:10Z"
}
```

### 4.6 HealthResponse

```json
{
	"status": "healthy",
	"queue_size": 0,
	"active_tasks": 0,
	"total_tasks": 10,
	"completed_tasks": 8,
	"failed_tasks": 2,
	"uptime": 0,
	"version": "1.0.0",
	"build_time": "2026-02-01T00:00:00Z",
	"git_commit": "abcdef",
	"git_branch": "main"
}
```

## 5. 接口列表

### 5.1 健康检查

**GET /health**

#### 说明

返回服务运行状态与统计数据。

#### 请求

- 无请求体

#### 响应

- 200：`HealthResponse`

#### 示例

请求：

```bash
curl http://localhost:8080/health
```

响应：

```json
{
	"code": 200,
	"data": {
		"status": "healthy",
		"queue_size": 0,
		"active_tasks": 0,
		"total_tasks": 10,
		"completed_tasks": 8,
		"failed_tasks": 2,
		"uptime": 0,
		"version": "1.0.0",
		"build_time": "2026-02-01T00:00:00Z",
		"git_commit": "abcdef",
		"git_branch": "main"
	}
}
```

---

### 5.2 上传项目

**POST /api/v1/upload**

#### 请求

- Content-Type：`multipart/form-data`
- 表单字段：
	- `file`（必填，`.tsp` 文件）

#### 约束

- 仅接受 `.tsp` 扩展名
- MIME 类型需为 ZIP/二进制
- 最大 100MB
- 上传完成后校验 ZIP 完整性

#### 行为

- 生成 `file_id`（SHA1）
- 解压到 `UPLOAD_DIR/{file_id}_extracted`
- 首次启动时由服务初始化基础资源与标准库

#### 响应

- 200：`UploadResponse`
- 400：上传参数或文件不符合要求
- 500：服务内部错误

#### 示例

请求：

```bash
curl -F "file=@./demo.tsp" http://localhost:8080/api/v1/upload
```

响应：

```json
{
	"code": 200,
	"data": {
		"file_id": "sha1_hex"
	}
}
```

---

### 5.3 创建构建任务

**POST /api/v1/build**

#### 请求体

```json
{
	"file_id": "sha1_hex"
}
```

#### 行为

- 校验 `file_id` 格式（40 位十六进制）
- 若已存在未结束任务，会复用并返回旧任务信息
- 否则创建新任务并加入队列

#### 响应

- 202：`BuildResponse`
- 400：`file_id` 不合法
- 404：文件不存在或已过期

#### 示例

请求：

```bash
curl -X POST http://localhost:8080/api/v1/build \
	-H "Content-Type: application/json" \
	-d '{"file_id":"sha1_hex"}'
```

响应：

```json
{
	"code": 202,
	"data": {
		"task_id": "uuid",
		"status": "排队中"
	}
}
```

---

### 5.4 查询任务状态

**GET /api/v1/build/{id}/status**

#### Path 参数

- `id`：`task_id`（UUID）

#### 响应

- 200：`Task`
- 400：`id` 不合法
- 404：任务不存在

#### 示例

请求：

```bash
curl http://localhost:8080/api/v1/build/{id}/status
```

响应：

```json
{
	"code": 200,
	"data": {
		"task_id": "uuid",
		"status": "处理中",
		"progress": 0,
		"estimated_time_remaining": 120,
		"current_step": "编译中",
		"error": null,
		"created_at": "2026-02-01T00:00:00Z",
		"updated_at": "2026-02-01T00:00:30Z",
		"retry_count": 0,
		"max_retries": 3,
		"build_duration": 120,
		"priority": 0
	}
}
```

---

### 5.5 查询任务事件

**GET /api/v1/build/{id}/events**

#### Path 参数

- `id`：`task_id`（UUID）

#### Query 参数

- `limit`：最大返回数量，默认 50，上限 200
- `offset`：偏移量，默认 0

#### 响应

- 200：`TaskEvent[]`
- 400：`id` 不合法
- 404：任务不存在

#### 示例

请求：

```bash
curl "http://localhost:8080/api/v1/build/{id}/events?limit=50&offset=0"
```

响应：

```json
{
	"code": 200,
	"data": [
		{
			"id": 1,
			"task_id": "uuid",
			"event_type": "log",
			"status": "处理中",
			"message": "step message",
			"worker_id": "worker-0",
			"created_at": "2026-02-01T00:00:10Z"
		}
	]
}
```

---

### 5.6 下载 APK

**GET /api/v1/build/{id}/download**

#### 说明

仅当任务状态为 `成功` 时可下载。

#### 响应

- 200：文件流（`application/vnd.android.package-archive`）
- 400：构建未成功
- 404：任务或文件不存在

文件名格式：`app-{task_id}.apk`

#### 示例

请求：

```bash
curl -L -o app.apk http://localhost:8080/api/v1/build/{id}/download
```

响应：

> 二进制文件流（非 JSON），保存为 `app.apk`

---

## 6. OpenAPI 与 Swagger

- OpenAPI JSON：`/api-docs/openapi.json`
- Swagger UI：`/swagger-ui/`

#### 示例

```bash
curl http://localhost:8080/api-docs/openapi.json
```

## 7. 附录：环境变量

| 变量名 | 默认值 | 说明 |
| --- | --- | --- |
| HOST | 0.0.0.0 | 监听地址 |
| PORT | 8080 | 监听端口 |
| UPLOAD_DIR | ./.tiec/uploads | 上传与解压目录 |
| DATABASE_PATH | ./.tiec/tasks.db | SQLite 数据库路径 |
| QUEUE_CAPACITY | 15 | 队列容量 |
| WORKER_COUNT | 1 | 工作线程数 |
| TASK_TIMEOUT | 900 | 单任务超时（秒） |
| CLEANUP_INTERVAL | 3600 | 清理任务间隔（秒） |
| HOURLY_IP_LIMIT | 20 | 每 IP 每小时限制（仅限部分 API） |

### build-test 测试参数（可选）

build-test 通过 **真实 API 流程**（/api/v1/upload、/api/v1/build）测试队列功能。优先使用 `test.json` 配置：

**test.json 示例：**

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
	"hourly_ip_limit": 60
}
```

**说明：**
- `file_path` 与 `file_paths` 可同时存在
- build-test 会先上传文件获得 file_id，再调用 /api/v1/build 创建真实构建任务
 - 任务重试倒计时会持续写入任务事件接口（event_type：`retry_countdown` / `retry_countdown_done`）

**环境变量（可选，覆盖或补充 test.json）：**

| 变量名 | 默认值 | 说明 |
| --- | --- | --- |
| BUILD_TEST_CONFIG | test.json | 配置文件路径 |
| BUILD_TEST_BASE_URL | http://127.0.0.1:8080 | 测试请求目标地址 |
| BUILD_TEST_FILE_PATH | 无 | 单个 .tsp 文件路径 |
| BUILD_TEST_FILE_PATHS | 无 | 多个 .tsp 文件路径（逗号分隔） |
| BUILD_TEST_INTERVAL_MS | 200 | 每个构建请求间隔（毫秒） |
| BUILD_TEST_TASK_TIMEOUT | 无 | build-test 模式下临时覆盖 TASK_TIMEOUT |
| BUILD_TEST_QUEUE_CAPACITY | 无 | build-test 模式下临时覆盖 QUEUE_CAPACITY |
