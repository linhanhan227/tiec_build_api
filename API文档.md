# TieCloud Build API 文档

## 概述

TieCloud Build API 是一个基于Rust和Actix Web开发的REST API服务，用于云端调用tiec编译器进行安卓项目打包。该API采用运行时文件复制策略管理编译器二进制文件，支持文件上传、异步构建任务管理、状态查询和结果下载。

## 技术栈

- **框架**: Actix Web 4.x
- **语言**: Rust 2021
- **构建脚本**: 自定义build.rs用于编译时文件复制
- **文档**: OpenAPI 3.0 (utoipa)
- **序列化**: Serde
- **异步**: Tokio

## 基础URL

```
http://localhost:8080/api/v1
```

## 响应格式

所有API响应都遵循统一的JSON格式：

**成功响应**:
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 200,
  "data": { ... }
}
```

**错误响应**:
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 400,
  "message": "Error description"
}
```

### 常见状态码

- `200`: 成功
- `202`: 已接受（异步操作）
- `400`: 请求参数错误
- `404`: 资源不存在
- `500`: 服务器内部错误

- `INTERNAL_ERROR`: 内部服务器错误
- `BAD_REQUEST`: 请求参数错误
- `NOT_FOUND`: 资源未找到
- `UNAUTHORIZED`: 未授权访问
- `UPLOAD_ERROR`: 文件上传错误

## API 端点

### 1. 健康检查

#### GET /health

检查API服务器的健康状态，包括队列状态和系统资源。

> 注意：健康检查不在 `/api/v1` 路径下，请直接访问 `/health`。

**响应**

成功 (200):
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 200,
  "data": {
    "status": "healthy",
    "queue_size": 0,
    "active_tasks": 0,
- API版本: v1
- 最后更新: 2026年1月31日 (租约队列 + 任务事件审计 + 事件查询接口)
    "failed_tasks": 5,
    "uptime": 3600,
    "version": "1.0.0"
  }
}
```

**健康指标**：
- `status`: 服务器状态 ("healthy" 或 "unhealthy")
- `queue_size`: 当前队列中的任务数量
- `active_tasks`: 正在处理的任务数量
- `total_tasks`: 总任务数
- `completed_tasks`: 已完成的任务数
- `failed_tasks`: 失败的任务数
- `uptime`: 服务器运行时间(秒)
- `version`: API版本

### 1.5. 任务清理

#### POST /api/v1/admin/cleanup

清理已完成、失败或过期的构建任务。支持按任务状态和时间进行清理。

**请求体**
```json
{
  "completed_max_age_hours": 24,
  "failed_max_age_hours": 168,
  "expired_max_age_hours": 1
}
```

**参数说明**：
- `completed_max_age_hours`: 清理超过此小时数的已完成任务 (默认: 24小时)
- `failed_max_age_hours`: 清理超过此小时数的失败任务 (默认: 168小时/7天)
- `expired_max_age_hours`: 清理超过此小时数的过期任务 (默认: 1小时)

**响应**
```json
{
  "code": 200,
  "data": {
    "completed_cleaned": 5,
    "failed_cleaned": 2,
    "expired_cleaned": 1,
    "total_cleaned": 8
  }
}
```

**响应字段**：
- `completed_cleaned`: 清理的已完成任务数量
- `failed_cleaned`: 清理的失败任务数量
- `expired_cleaned`: 清理的过期任务数量
- `total_cleaned`: 总清理数量

### 2. 文件上传

#### POST /api/v1/upload

上传安卓项目文件（.tsp ZIP格式）。上传后，系统会自动解压ZIP文件并将stdlib/android目录复制到项目根目录下的"基本库"文件夹中。

**请求**
- Content-Type: `multipart/form-data`
- Body: 文件字段 `file`

**处理流程**
1. 验证文件类型和完整性
2. 保存原始ZIP文件
3. 解压ZIP文件到临时目录
4. 复制stdlib/android目录到解压目录的"基本库"文件夹
5. 返回文件ID用于后续构建

**验证规则**
- 文件扩展名: `.tsp`
- MIME类型: `application/zip` 或 `application/x-zip-compressed`
- 文件大小: ≤ 100MB
- ZIP完整性: 尝试解压验证

**响应**

成功 (200):
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 200,
  "data": {
    "file_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

错误 (400/500):
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 400,
  "message": "Invalid file type. Only .tsp (zip) files are allowed."
}
```

### 3. 创建构建任务

#### POST /api/v1/build

提交构建任务到队列。系统会根据当前运行环境的操作系统和架构自动选择合适的tiec编译器二进制文件进行构建。

**构建过程**
1. 解压上传的TSP文件
2. 根据系统架构从可执行文件同目录的tiecc文件夹中复制合适的tiec编译器：
   - macOS: tiecc/macos/tiec
   - Linux x86_64: tiecc/linux_x86_64/tiec
   - Linux ARM64: tiecc/linux_arm64-v8a/tiec
   - Windows x86_64: tiecc/win_x86_64/tiec.exe
3. 将编译器复制到工作目录并设置执行权限
4. 执行构建命令：`{tiec_path} -o {project_dir}/build --platform android --android.gradle --android.app.config build/project.json --release --log-level error --dir {project_dir}`
   - macOS: `tiecc/macos/tiec`
   - Linux x86_64: `tiecc/linux_x86_64/tiec`
   - Linux ARM64: `tiecc/linux_arm64-v8a/tiec`
   - Windows x86_64: `tiecc/win_x86_64/tiec.exe`
5. 在构建目录中查找生成的APK文件

**请求**
```json
{
  "file_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**参数说明**
- `file_id` (string, required): 上传文件的ID

**响应**

成功 (202):
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 202,
  "data": {
    "task_id": "550e8400-e29b-41d4-a716-446655440001",
    "status": "排队中"
  }
}
```

### 4. 查询任务事件

#### GET /api/v1/build/{id}/events

查询指定任务的事件审计记录（支持分页）。

**请求参数**：

- `id` (Path)：任务ID
- `limit` (Query，可选)：返回事件数量（默认50，最大200）
- `offset` (Query，可选)：偏移量（默认0）

**请求示例**：

```
GET /api/v1/build/550e8400-e29b-41d4-a716-446655440001/events?limit=50&offset=0
```

**响应示例**：

```json
{
  "code": 200,
  "data": [
    {
      "id": 1,
      "task_id": "550e8400-e29b-41d4-a716-446655440001",
      "event_type": "enqueued",
      "status": "排队中",
      "message": "Task enqueued",
      "worker_id": null,
      "created_at": "2026-01-31T10:00:00Z"
    },
    {
      "id": 2,
      "task_id": "550e8400-e29b-41d4-a716-446655440001",
      "event_type": "leased",
      "status": "处理中",
      "message": "Task leased",
      "worker_id": "worker-0",
      "created_at": "2026-01-31T10:00:01Z"
    }
  ]
}
```

### 5. 查询构建状态

#### GET /api/v1/build/{taskId}/status

查询指定构建任务的详细状态。

**路径参数**
- `taskId` (string): 任务ID

**请求示例**：

```
GET /api/v1/build/550e8400-e29b-41d4-a716-446655440001/status
```

**响应**

成功 (200):
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 200,
  "data": {
    "task_id": "550e8400-e29b-41d4-a716-446655440001",
    "status": "处理中",
    "progress": 45,
    "estimated_time_remaining": 120,
    "current_step": "Compiling resources...",
    "error": null,
    "created_at": "2026-01-26T10:00:00Z",
    "updated_at": "2026-01-26T10:05:00Z"
  }
}
```

**状态枚举**
- `排队中`: QUEUED
- `处理中`: PROCESSING
- `成功`: SUCCESS
- `失败`: FAILED
- `已取消`: CANCELLED

### 6. 下载构建结果

#### GET /api/v1/build/{taskId}/download

下载成功构建的APK文件。

**路径参数**
- `taskId` (string): 任务ID

**请求示例**：

```
GET /api/v1/build/550e8400-e29b-41d4-a716-446655440001/download
```

**响应头**
- Content-Type: `application/vnd.android.package-archive`
- Content-Disposition: `attachment; filename="app-{taskId}.apk"`

**响应**
- 二进制APK文件

**错误响应**

400 (构建未完成):
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 400,
  "message": "Build not completed or task cancelled"
}
```

404 (任务不存在):
```
Content-Type: application/json; charset=utf-8
X-API-Version: v1

{
  "code": 404,
  "message": "Task not found"
}
```

## 数据模型

### ApiResponse<T>
```json
{
  "code": 200,
  "data": "T"
}
```

### Task
```json
{
  "task_id": "uuid",
  "status": "TaskStatus",
  "progress": 0,
  "estimated_time_remaining": 120,
  "current_step": "string",
  "error": "string",
  "created_at": "2026-01-26T10:00:00Z",
  "updated_at": "2026-01-26T10:05:00Z",
  "retry_count": 0,
  "max_retries": 3
}
```

### TaskStatus
枚举值: `"排队中"`, `"处理中"`, `"成功"`, `"失败"`, `"已取消"`

### TaskEvent
```json
{
  "id": 1,
  "task_id": "uuid",
  "event_type": "string",
  "status": "TaskStatus",
  "message": "string",
  "worker_id": "string",
  "created_at": "2026-01-31T10:00:00Z"
}
```

### BuildRequest
```json
{
  "file_id": "string"
}
```

### BuildResponse
```json
{
  "task_id": "string",
  "status": "string"
}
```

### UploadResponse
```json
{
  "file_id": "string"
}
```

## 构建过程详解

### 文件处理流程

1. **文件上传**: 用户上传 `.tsp` ZIP文件，服务器生成 `file_id` 并保存文件
2. **构建请求**: 用户提交构建请求，服务器生成唯一的 `task_id`
3. **文件解压**: 服务器解压ZIP文件到工作目录
4. **目录重命名**: 为避免冲突和便于管理，解压后的项目目录重命名为 `task_id`
5. **编译器准备**: 根据系统架构从 `tiecc/` 目录复制相应的编译器二进制文件到工作目录
6. **构建执行**: 调用复制的 `tiec` 构建工具处理项目
7. **结果保存**: 构建成功后保存APK文件

### 工作目录结构

```
./.tiec/tie_build_{task_id}/
├── source/           # 原始解压目录（临时）
├── {task_id}/        # 重命名后的项目目录
└── output/
    └── app.apk       # 构建结果
```

### 目录重命名逻辑

- **优先方案**: 如果ZIP文件只包含一个根目录，则将该目录重命名为 `task_id`
- **备选方案**: 如果ZIP文件包含多个文件/目录或没有根目录，则使用原始解压目录
- **目的**: 确保每个构建任务有唯一的、可识别的项目目录，避免多任务间的冲突

### 编译器文件管理

API服务器采用运行时文件复制策略管理tiec编译器二进制文件：

1. **构建时复制**: 使用自定义build.rs脚本在编译时将所有平台的tiec二进制文件从`src/tiecc/`复制到构建输出目录的`tiecc/`文件夹
2. **运行时加载**: 服务器启动时无需特殊处理，编译器文件已位于可执行文件同目录
3. **动态选择**: 根据运行环境的操作系统和架构自动选择合适的编译器文件
4. **权限设置**: 复制编译器文件时自动设置执行权限

**优势**：
- 减小可执行文件大小（不再嵌入二进制）
- 便于编译器版本更新
- 支持跨平台部署
- 简化构建流程

## 后端二进制文件集成

构建过程调用 `tiec` 二进制文件（位于可执行文件同目录的 `tiecc/{platform}/tiec` 路径）。

### 命令示例
```bash
tiec build -p /path/to/source -o /path/to/output.apk
```

### 资源限制
- CPU: 无特定限制（可通过cgroup配置）
- 内存: 1GB 限制
- 时间: 15分钟超时

## 后台任务

### 构建工作器

API采用异步任务队列处理构建请求，支持高并发和可靠的任务执行：

**队列机制**：
- **持久化队列**: 基于数据库的租约队列（Lease Queue），支持进程重启恢复
- **队列容量**: 100个待处理任务（可配置）
- **处理模式**: 多工作器并发拉取，租约续期防重复消费

**任务状态流转**：
```
排队中 → 处理中 → 成功/失败
  ↘ 重试排队 → 处理中
  ↘ 已取消
```

**并发控制**：
- 支持配置多个工作器并行处理任务
- 任务按优先级和创建时间调度
- 通过租约避免多实例重复处理
- 支持水平扩展（多进程部署）

### 清理任务

- **执行频率**: 每小时自动执行
- **清理规则**: 删除超过24小时的临时文件和构建目录
- **安全措施**: 只清理 `./.tiec/` 目录下的文件，避免误删重要数据

## 任务队列管理

### 队列架构

**核心组件**：
- **任务存储**: 数据库持久化任务状态，内存缓存加速读取
- **租约队列**: 数据库字段控制出队与租约（`lease_until`）
- **状态同步**: 实时更新任务进度、状态与审计事件

**队列特性**：
- **容量限制**: 100个待处理任务（可配置）
- **可靠消费**: 租约过期自动回收，避免任务丢失
- **并发安全**: 数据库原子更新保证任务只被一个工作器租约
- **实时更新**: 任务进度、状态与事件实时持久化

### 任务生命周期

1. **任务创建**: 用户提交构建请求，生成唯一task_id
2. **队列入队**: 任务写入数据库并设置 `next_run_at`
3. **租约拉取**: 工作器通过租约获取任务（避免重复消费）
4. **处理开始**: 任务状态变为"处理中"并续租
5. **进度更新**: 实时更新进度(0-100%)和当前步骤
6. **失败重试**: 失败时按指数退避重新排队
7. **完成处理**: 任务状态设为"成功"或"失败"
8. **结果保存**: 成功时保存APK文件路径，失败时记录错误信息

### 错误处理

**任务失败场景**：
- 文件解压失败
- 编译器复制失败
- 构建命令执行失败
- 超时（可配置，默认15分钟）
- 系统资源不足

**失败处理策略**：
- **自动重试**: 任务失败时自动重试（最多3次）
- **重试记录**: 记录重试次数和每次失败的原因
- **最终失败**: 达到最大重试次数后标记为永久失败
- **错误详情**: 保存完整的错误信息和堆栈跟踪
- **资源清理**: 清理临时文件，避免资源泄漏
- **继续处理**: 单个任务失败不影响其他任务的处理

### 队列监控

**实时指标**：
- 当前队列长度
- 活跃任务数量
- 任务完成率
- 平均处理时间

**日志记录**：
- 任务开始/完成事件
- 错误详情和堆栈跟踪
- 性能指标统计

### 任务事件审计

系统会将关键事件写入 `task_events` 表，用于审计与排障：

- `enqueued`: 任务入队
- `leased`: 工作器租约获取
- `status_change`: 状态/进度变化
- `retry_scheduled`: 失败重试调度
- `cancelled`: 因容量或策略取消

事件包含：`task_id`、`event_type`、`status`、`message`、`worker_id`、`created_at`。

### 指标上报

基础指标通过健康检查接口返回（`GET /health`）：

- 队列长度
- 活跃任务数量
- 总任务数/成功/失败
- 版本与构建信息

## 部署和配置

### 环境变量

**服务器配置**：
- `HOST`: 服务器主机 (默认: 0.0.0.0)
- `PORT`: 服务器端口 (默认: 8080)
- `UPLOAD_DIR`: 上传目录 (默认: ./.tiec/uploads)
- `DATABASE_PATH`: SQLite数据库路径 (默认: ./.tiec/tasks.db)
  - **自动创建**: 如果数据库文件不存在，系统会自动创建数据库文件和必要的目录结构

**队列配置**：
- `QUEUE_CAPACITY`: 任务队列容量 (默认: 15)
- `WORKER_COUNT`: 并发工作器数量 (默认: 1)
- `TASK_TIMEOUT`: 任务超时时间(秒) (默认: 900, 15分钟)
- `CLEANUP_INTERVAL`: 清理任务间隔(秒) (默认: 3600, 1小时)

**限流配置**：
- `HOURLY_IP_LIMIT`: 上传/构建相关接口每小时每IP最大请求数 (默认: 20)

**日志配置**：
- `RUST_LOG`: 日志级别 (默认: info)

### 运行
```bash
cargo run
```

### Swagger UI
访问 `http://localhost:8080/swagger-ui/` 查看交互式API文档。

## 安全考虑

- 文件大小和类型严格验证
- ZIP文件完整性检查
- 临时文件定期清理
- 子进程资源限制
- **编译器文件安全性**：编译器二进制文件独立存储，便于安全审计和病毒扫描
- **接口频率限制**：上传与构建相关接口同一IP每小时最多 `HOURLY_IP_LIMIT` 次，超过返回 429 提示

## 监控和日志

### 构建日志

- 所有构建日志通过tokio异步捕获
- 错误日志包含详细堆栈跟踪
- 任务状态实时更新

### 队列监控

**实时指标**：
- **队列状态**: 当前排队任务数量、活跃任务数
- **处理统计**: 任务完成率、平均处理时间、失败率
- **资源使用**: CPU/内存使用率、磁盘空间占用
- **性能指标**: 队列吞吐量、响应时间分布

**日志事件**：
- 任务入队/出队事件
- 工作器启动/停止事件
- 队列满载警告
- 任务超时告警
- 清理任务执行记录

### 健康检查

API提供以下健康检查端点：
- `GET /health`: 基础健康检查（不在 `/api/v1` 下）
- `GET /metrics`: 详细性能指标（如需对接 Prometheus/OTel 需自行启用）

**健康指标**：
- 队列处理能力
- 磁盘空间充足性
- 编译器文件完整性
- 网络连接状态

## 部署说明

### 单二进制部署

该API服务器支持单二进制部署，资源文件在编译时嵌入二进制，首次运行时自动释放到可执行文件同目录的 `.tiec/`：

```bash
# 构建发布版本
cargo build --release

# 运行单二进制文件
./target/release/tie_api_server
```

**优势**：
- 无需额外文件依赖
- 简化部署流程
- 减少分发复杂性
- **跨平台兼容**：自动检测运行环境并使用合适的编译器
- **本地文件管理**：资源文件存储在可执行文件同目录的 `./.tiec/` 下
- **易于更新**：重新构建即可更新内嵌资源

**目录结构**：
```
./
├── tie_api_server                    # 主可执行文件
├── tiecc/                            # 编译器目录（构建时自动复制）
│   ├── macos/
│   │   └── tiec
│   ├── linux_x86_64/
│   │   └── tiec
│   ├── linux_arm64-v8a/
│   │   └── tiec
│   └── win_x86_64/
│       └── tiec.exe
└── .tiec/
    ├── uploads/                      # 上传文件存储
    └── tie_build_{task_id}/          # 构建工作目录
        ├── source/                   # 解压后的源文件
        ├── {task_id}/               # 项目目录
        └── output/                   # 构建输出
```

### 编译器更新

由于采用运行时文件复制策略，编译器更新变得非常简单：

1. **替换编译器文件**：直接替换 `tiecc/` 目录下相应平台的编译器文件
2. **重启服务**：无需重新编译整个应用程序，只需重启服务器即可使用新版本编译器
3. **版本管理**：可以为不同版本的编译器创建备份，便于快速回滚

**目录结构**：
```
./
├── tie_api_server                    # 主可执行文件
└── .tiec/                            # 运行时自动创建
  ├── tiecc/                        # 编译器目录（运行时释放）
  │   ├── macos/
  │   │   └── tiec
  │   ├── linux_x86_64/
  │   │   └── tiec
  │   ├── linux_arm64-v8a/
  │   │   └── tiec
  │   └── win_x86_64/
  │       └── tiec.exe
  ├── stdlib/                       # 标准库目录（运行时释放）
  └── uploads/                      # 上传文件目录
这种方式比嵌入式二进制更新更加灵活高效。

## 扩展性

### 队列扩展

**水平扩展策略**：
- **多进程部署**: 每个API实例独立处理任务队列
- **负载均衡**: 通过反向代理分发请求到多个实例
- **队列分片**: 支持将不同类型的任务路由到专门的工作器

**性能优化**：
- **并发工作器**: 可配置多个工作器并行处理任务
- **资源池化**: 复用编译器进程和临时目录
- **缓存机制**: 缓存常用依赖和编译器文件

### 存储扩展

- 文件存储可配置为S3/MinIO对象存储
- 支持Redis作为任务队列后端替代内存队列
- 数据库集成用于长期任务历史存储

### 实时通信

- WebSocket可用于实时状态推送
- Server-Sent Events (SSE)用于单向状态更新
- 消息队列集成支持分布式架构

## 版本信息

- API版本: v1
- 最后更新: 2026年1月31日 (租约队列 + 任务事件审计 + 事件查询接口)