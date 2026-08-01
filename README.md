# rust-lab-sample-flow

实验样本流转 API 服务，用于管理科研样本从采集、入库、处理、转移到归档的全过程，覆盖批次、样本、状态变更和操作日志，重点考察 Rust 后端、SQLite 建模、状态机约束和 REST API 设计。

纯后端 REST API 服务（Rust + axum + SQLite），监听 `18101` 端口，所有请求和响应均为 JSON，不包含任何前端页面或图形界面。

## 启动命令

```bash
cargo run          # 开发模式，监听 0.0.0.0:18101
cargo test         # 运行全部单元测试（状态机 / 校验 / 异常 / 查询过滤）
cargo run --release
```

数据库表结构在程序启动时自动创建，无需手动执行任何 SQL 文件。

## SQLite 文件位置

默认：`./data/sample_flow.db`（目录不存在时自动创建）。

可通过环境变量覆盖：

```bash
SAMPLE_FLOW_DB=/path/to/another.db cargo run
```

## 目录结构

```
├── Cargo.toml
├── data/sample_flow.db     # 运行时生成的 SQLite 数据库
└── src
    ├── main.rs             # 入口：初始化数据库、绑定 18101、启动 HTTP 服务
    ├── db.rs               # SQLite 连接与建表 Schema（启动时自动初始化）
    ├── error.rs            # 统一错误结构 ApiError（code / message / details）
    ├── models.rs           # 实体、请求体模型 + 状态机规则 can_transition
    ├── repo.rs             # 数据库访问层（SQL 增删改查）
    ├── service.rs          # 业务逻辑层（校验、状态流转、日志记录）+ 单元测试
    └── routes.rs           # 路由层（axum handler、路径与 JSON 绑定）
```

## 接口分组

所有接口前缀 `/api`，请求/响应均为 JSON。

### 批次（batches）

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/batches` | 创建批次（batch_no、project_name、manager、remark） |
| GET | `/api/batches` | 批次列表 |
| GET | `/api/batches/:batch_no` | 批次详情 |
| GET | `/api/batches/:batch_no/samples` | 按批次查询样本 |

### 样本（samples）

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/samples` | 登记单个样本（初始状态 collected） |
| POST | `/api/batches/:batch_no/samples/bulk-import` | 批量导入样本（单事务，全部成功或全部回滚，详见下文） |
| GET | `/api/samples/:sample_no` | 样本详情（含存放位置） |
| GET | `/api/samples` | 组合筛选：`batch_no`、`status`、`sample_type`、`region`、`freezer_no`、`shelf`、`slot` |

#### 批量导入说明

- 一次请求为指定批次导入多条样本，请求体：`{"operator": "...", "samples": [{"sample_no": "...", "sample_type": "...", "location": {"region","freezer_no","shelf","slot"}?}]}`，其中 `location` 可选。
- **原子性**：整个导入在单个事务中执行。任何一条样本编号重复（409 `DUPLICATE_SAMPLE`）、样本类型为空或位置字段缺失/空白（400 `VALIDATION_ERROR`，提示第几条及原因），都会**整批回滚**，不会出现部分成功。
- 位置一旦提供，`region` / `freezer_no` / `shelf` / `slot` 四字段都必须非空；相同位置自动去重复用。
- 导入成功后，每个样本自动写入一条初始操作日志，`action` 统一为 `created`，`operator` 取请求中的操作者（缺省 `system`）。
- 批次不存在返回 404 `BATCH_NOT_FOUND`，不写入任何数据。

### 状态与位置

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| PUT | `/api/samples/:sample_no/status` | 状态更新（`status`、`operator`、`note`） |
| PUT | `/api/samples/:sample_no/location` | 位置变更（region / freezer_no / shelf / slot，自动去重复用位置记录） |
| GET | `/api/locations` | 位置查询：`region`、`freezer_no`、`shelf`、`slot` 四个查询参数必填 |

#### 位置占用与查询规则

- **占用校验**：同一个位置（区域 + 冰箱编号 + 层架 + 格位）同一时间只允许存放**一个未归档样本**；样本归档时自动释放位置——清空样本当前位置并向该位置历史写入一条 `direction=out`、`to_location=null` 的迁出记录。位置变更和批量导入都会校验，冲突返回 409 `LOCATION_OCCUPIED`（`details` 含占用者编号）；批量导入中同一请求内两条样本使用同一位置同样整批回滚。位置字段在入库前会去除首尾空格，带空格的字段值无法绕过占用校验。
- **变更日志**：每次位置变更写入操作日志，`note` 同时包含旧位置和新位置，如 `位置变更: A区/F-01/1/A1 -> B区/F-02/2/B3`（首次设置旧位置显示 `(未设置)`）。
- **位置查询**：`GET /api/locations?region=..&freezer_no=..&shelf=..&slot=..` 返回：
  - `location`：位置本身；
  - `current_sample`：当前占用该位置的未归档样本（无占用为 `null`）；
  - `recent_movements`：该位置最近 10 次迁入迁出记录（倒序），每条含 `sample_no`、`direction`（`in`=迁入 / `out`=迁出）、`from_location`、`to_location`、`operator`、`created_at`。
- 四元组参数缺失或为空返回 400 `VALIDATION_ERROR`；位置不存在返回 404 `LOCATION_NOT_FOUND`。

### 异常标记（exception flags）

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/samples/:sample_no/exceptions` | 新增异常标记（`contamination` / `label_missing` / `temperature_abnormal` / `other`） |
| GET | `/api/samples/:sample_no/exceptions` | 样本的异常列表 |
| POST | `/api/samples/:sample_no/exceptions/:id/resolve` | 解除异常（重复解除返回 409） |

### 流转日志（operation logs）

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/samples/:sample_no/logs` | 单个样本的完整流转日志（登记、状态变更、位置变更、异常标记/解除） |

## 状态流转规则

状态只能在 `collected`、`stored`、`processing`、`transferred`、`archived` 之间**链式单向**流转，禁止跳过关键状态、禁止回退：

```
collected → stored → processing → transferred → archived
```

- 非法跳步（如 `collected → archived`）返回 `409 INVALID_STATE_TRANSITION`，`details` 中带 `from` / `to` / `rule`。
- 非法状态值返回 `400 VALIDATION_ERROR`。
- `archived` 为终态，不允许任何转出。
- 每次状态变更都会写入操作日志（操作者、动作、说明、时间）。

## 统一错误结构

```json
{
  "error": {
    "code": "INVALID_STATE_TRANSITION",
    "message": "不允许从 collected 直接流转到 archived",
    "details": { "from": "collected", "to": "archived", "rule": "collected -> stored -> processing -> transferred -> archived" }
  }
}
```

常见错误码：`VALIDATION_ERROR`(400，含空白 operator、空查询参数值)、`INVALID_JSON` / `INVALID_PATH_PARAM` / `INVALID_QUERY_PARAM`(400)、`ROUTE_NOT_FOUND`(404)、`METHOD_NOT_ALLOWED`(405)、`BATCH_NOT_FOUND` / `SAMPLE_NOT_FOUND` / `EXCEPTION_NOT_FOUND` / `LOCATION_NOT_FOUND`(404)、`DUPLICATE_BATCH` / `DUPLICATE_SAMPLE` / `INVALID_STATE_TRANSITION` / `ALREADY_RESOLVED` / `LOCATION_OCCUPIED`(409)、`INTERNAL_ERROR`(500)。

未匹配的路径、不支持的请求方法、路径参数或查询字符串解析失败时，同样返回上述统一 JSON 错误结构。

## 验证样本流转闭环

启动服务后执行以下 curl 序列，可验证「采集 → 入库 → 处理 → 转移 → 归档」的完整闭环：

```bash
B=http://localhost:18101/api

# 1. 创建批次
curl -X POST $B/batches -H 'Content-Type: application/json' \
  -d '{"batch_no":"B001","project_name":"肿瘤标志物研究","manager":"张三"}'

# 2. 登记样本（初始状态 collected）
curl -X POST $B/samples -H 'Content-Type: application/json' \
  -d '{"sample_no":"S0001","batch_no":"B001","sample_type":"blood","operator":"alice"}'

# 3. 验证跳步被拒（应返回 409 INVALID_STATE_TRANSITION）
curl -X PUT $B/samples/S0001/status -H 'Content-Type: application/json' \
  -d '{"status":"archived","operator":"alice"}'

# 4. 设置存放位置
curl -X PUT $B/samples/S0001/location -H 'Content-Type: application/json' \
  -d '{"region":"A区","freezer_no":"F-01","shelf":"3","slot":"A5","operator":"alice"}'

# 5. 沿链路逐级流转（每步应返回 200）
for s in stored processing transferred archived; do
  curl -X PUT $B/samples/S0001/status -H 'Content-Type: application/json' \
    -d "{\"status\":\"$s\",\"operator\":\"bob\"}"
done

# 6. 查看完整流转日志（应包含 register / location_change / 4 次 status_update）
curl $B/samples/S0001/logs

# 7. 按状态 / 位置 / 批次检索
curl "$B/samples?status=archived"
curl "$B/samples?region=A区&freezer_no=F-01"
curl $B/batches/B001/samples
```
