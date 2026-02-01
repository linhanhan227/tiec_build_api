# TieCloud Build API

面向 TieCloud 的构建服务后端，提供 .tsp 项目上传、构建任务创建、状态/事件查询与 APK 下载能力，并内置 OpenAPI/Swagger 文档。

## 功能概览

- 上传 .tsp（ZIP）项目文件并生成 `file_id`
- 创建构建任务并返回 `task_id`
- 查询任务状态与构建事件
- 构建成功后下载 APK
- 健康检查与运行状态统计
- OpenAPI/Swagger 文档与速率限制

## 快速开始

1) 构建

```bash
cargo build --release
```

2) 运行

```bash
./target/release/tie_api_server
```

3) 查看 Swagger 文档

打开浏览器访问：

```
http://localhost:8080/swagger-ui/
```

## 配置

通过环境变量覆盖默认值：

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

## 目录结构（核心）

- src/api：API 处理器
- src/worker：任务执行与清理
- src/state：内存状态与队列
- src/database：SQLite 访问
- src/middleware：限流与封禁

## API 文档

详细接口说明见 [API文档.md](API文档.md)。

补充文档：

- [项目完整文档.md](项目完整文档.md)
- [代码深度解析.md](代码深度解析.md)

同时提供 OpenAPI JSON：

- /api-docs/openapi.json
- /swagger-ui/

## 速率限制说明

- 秒级限流：每 IP 每秒 120 次，请求超限将被封禁 7 天
- 小时级限流：对上传与构建相关接口生效，默认每 IP 每小时 20 次

## License

内部使用或按项目要求指定。
