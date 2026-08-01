# rust-lab-sample-flow

实验样本流转管理后端服务。使用 Rust 编写的纯 REST API，记录科研样本从**采集 → 入库 → 处理 → 转移 → 归档**的完整流转轨迹。无前端页面、无后台管理界面，仅提供可调用的 JSON 接口。

技术栈：`axum` 0.8 + `tokio` + `rusqlite`（bundled SQLite）+ `r2d2` 连接池 + `serde` + `chrono`。

## 启动命令

```bash
# 编译并启动（首次会自动下载依赖、编译 SQLite）
cargo run

# 运行测试
cargo test

# 生产构建
cargo build --release && ./target/release/rust-lab-sample-flow
```

服务监听 **`0.0.0.0:18101`**。启动日志会打印实际地址与数据库文件路径。

## SQLite 文件位置

- 数据库文件：`data/lab_sample_flow.db`（相对于启动时的工作目录）。
- 启动时自动创建 `data/` 目录，并**自动初始化全部数据表**（`CREATE TABLE IF NOT EXISTS`），无需手动执行任何 SQL 文件。
- 删除该文件即可重置全部数据。

## 目录结构

```
rust-lab-sample-flow/
├── Cargo.toml
├── README.md
├── data/
│   └── lab_sample_flow.db      # 运行时自动生成
├── src/
│   ├── main.rs                 # 启动入口：建目录、初始化 DB、监听 18101
│   ├── lib.rs                  # 对外导出（供集成测试复用）
│   ├── db.rs                   # 连接池 + 建表（自动初始化 schema）
│   ├── models.rs               # 请求/响应数据结构、状态常量
│   ├── error.rs                # 统一错误结构与 HTTP 映射
│   ├── repo.rs                 # 数据库访问层（纯 SQL 读写）
│   ├── service.rs              # 业务逻辑层（校验、状态机、事务）
│   └── routes.rs               # 路由与 HTTP 处理层
└── tests/
    └── api.rs                  # 集成测试（22 个用例）
```

分层职责：`routes.rs`（路由/HTTP）→ `service.rs`（业务逻辑与校验）→ `repo.rs`（数据访问）→ `db.rs`（连接与建表），错误类型集中在 `error.rs`。

## 核心资源

| 资源 | 表 | 字段 |
| --- | --- | --- |
| 样本批次 | `batches` | 批次编号、项目名称、负责人、备注、创建时间 |
| 单个样本 | `samples` | 样本编号、所属批次、样本类型、当前状态、存放位置、最近处理时间 |
| 操作日志 | `operation_logs` | 操作者、动作、说明、时间 |
| 异常标记 | `exceptions` | 异常类型、描述、状态(open/resolved)、上报人/解除人及时间 |
| 存放位置 | `locations` / `location_movements` | 区域、冰箱编号、层架、格位；迁入迁出记录 |

## 状态流转规则

样本状态只能在以下五个状态间**逐级正向推进**，禁止跳跃、禁止回退：

```
collected → stored → processing → transferred → archived
```

- 例如 `collected → archived` 会被拒绝（`INVALID_TRANSITION`）。
- 回退（如 `stored → collected`）会被拒绝。
- 非法状态值会被拒绝（`VALIDATION_ERROR`）。
- 样本存在**未解除的异常标记**时，禁止状态流转（`CONFLICT`）。
- 归档（`archived`）后自动释放其存放位置占用，并记录一次位置迁出。

位置占用规则：同一存放位置在同一时刻只能被一个**非归档**样本占用；登记、位置变更、批量导入均会校验占用冲突（`CONFLICT`）。

## 存放位置管理

- **占用唯一性**：同一「区域 + 冰箱编号 + 层架 + 格位」在同一时刻只能存放一个**未归档**样本。样本一旦归档即释放其位置（`samples.location_id` 置空并记录一次迁出），该格位可被后续样本使用。
- **位置变更日志**：`PATCH /api/samples/{sample_no}/location` 会写入一条 `action=location_changed` 的操作日志，说明中同时包含**旧位置和新位置**（格式 `位置变更: 区域/冰箱/层架/格位 -> 区域/冰箱/层架/格位`；无旧位置时旧位置显示为 `无`）。若未显式传入 `note`，则自动生成上述说明。同时在 `location_movements` 表记录旧位置的迁出与新位置的迁入。
- **位置查询**：`GET /api/locations?area=&freezer=&shelf=&slot=`（四字段必填）返回该位置信息、当前占用样本 `current_sample`（无占用则该字段省略），以及该位置**最近 10 次**迁入/迁出记录 `recent_movements`（按时间倒序，`direction` 为 `in`/`out`）。

## 接口分组

所有请求/响应均为 JSON。基础地址 `http://127.0.0.1:18101`。

### 批次
- `POST /api/batches` — 创建批次
- `POST /api/batches/{batch_no}/samples` — 样本登记
- `POST /api/batches/{batch_no}/samples/bulk-import` — 批量导入样本（整批事务，任一失败则全部回滚）

### 样本状态与位置
- `PATCH /api/samples/{sample_no}/status` — 样本状态更新
- `PATCH /api/samples/{sample_no}/location` — 位置变更

### 异常
- `POST /api/samples/{sample_no}/exceptions` — 异常标记
- `PATCH /api/exceptions/{exception_id}/resolve` — 异常解除

### 查询
- `GET /api/samples?batch_no=` — 按批次查询样本
- `GET /api/samples?status=` — 按状态筛选样本
- `GET /api/samples?area=&freezer=&shelf=&slot=` — 按位置检索样本（可任意组合）
- `GET /api/samples?sample_type=` — 按样本类型筛选
- `GET /api/samples/{sample_no}` — 查看单个样本
- `GET /api/samples/{sample_no}/logs` — 查看单个样本完整流转日志
- `GET /api/locations?area=&freezer=&shelf=&slot=` — 存放位置检索（当前占用样本 + 最近迁移记录）

### 其他
- `GET /health` — 健康检查

## 批量导入样本

`POST /api/batches/{batch_no}/samples/bulk-import` 一次请求向指定批次导入多条样本。

请求体：

```json
{
  "samples": [
    { "sample_no": "SMP-101", "sample_type": "blood",
      "location": { "area": "A", "freezer": "F1", "shelf": "1", "slot": "01" } },
    { "sample_no": "SMP-102", "sample_type": "urine" }
  ]
}
```

- `location` 可选；若提供则四字段（区域/冰箱编号/层架/格位）必须齐全。

**整批原子性（全有或全无）**：整个导入在单个 SQLite 事务内执行，只要出现以下任一情况，**整批回滚，不会出现部分成功**：

- 任一 `sample_no` 与库中已有样本重复，或请求内部相互重复；
- 任一 `sample_type` 为空（或纯空白）；
- 任一 `location` 字段缺失/为空，或请求内部位置重复，或目标位置已被非归档样本占用；
- 指定 `batch_no` 不存在（返回 `404 NOT_FOUND`）。

**自动初始日志**：导入成功后，每个样本都会自动写入一条 `action` 为 `created` 的初始操作日志（说明为「批量导入」），初始状态为 `collected`。

成功响应（`201 Created`）：

```json
{ "imported": 2, "samples": [ /* 每条样本的完整信息 */ ] }
```

## 统一错误结构

所有错误（含 404 / 405 / JSON 解析失败）都返回统一结构：

```json
{ "code": "INVALID_TRANSITION", "message": "禁止从 'collected' 直接变更为 'archived' ...", "details": null }
```

| 错误码 | HTTP | 含义 |
| --- | --- | --- |
| `VALIDATION_ERROR` | 400 | 输入校验失败（空字段、非法状态、空查询参数、JSON 解析失败等） |
| `INVALID_TRANSITION` | 400 | 非法状态流转（跳跃/回退/重复） |
| `NOT_FOUND` | 404 | 资源不存在或接口不存在 |
| `METHOD_NOT_ALLOWED` | 405 | 请求方法不被允许 |
| `CONFLICT` | 409 | 编号重复、位置被占用、存在未解除异常等冲突 |
| `INTERNAL_ERROR` | 500 | 内部错误 |

## 验证样本流转闭环

启动服务后执行以下步骤，可完整验证「采集 → 归档」闭环：

```bash
B=http://127.0.0.1:18101

# 1. 创建批次
curl -s -X POST $B/api/batches -H 'content-type: application/json' \
  -d '{"batch_no":"BATCH-001","project_name":"COVID","owner":"alice","note":"首批"}'

# 2. 登记样本（带存放位置）
curl -s -X POST $B/api/batches/BATCH-001/samples -H 'content-type: application/json' \
  -d '{"sample_no":"SMP-001","sample_type":"blood","location":{"area":"A","freezer":"F1","shelf":"1","slot":"01"}}'

# 3. 逐级流转：collected → stored → processing → transferred → archived
for st in stored processing transferred archived; do
  curl -s -X PATCH $B/api/samples/SMP-001/status -H 'content-type: application/json' \
    -d "{\"status\":\"$st\",\"operator\":\"bob\"}"
  echo
done

# 4. 反例：跳过关键状态应被拒绝（返回 INVALID_TRANSITION）
curl -s -X POST $B/api/batches/BATCH-001/samples -H 'content-type: application/json' \
  -d '{"sample_no":"SMP-002","sample_type":"blood"}' >/dev/null
curl -s -X PATCH $B/api/samples/SMP-002/status -H 'content-type: application/json' \
  -d '{"status":"archived","operator":"bob"}'
echo

# 5. 查看完整流转日志（应包含 created + 4 次状态变更）
curl -s $B/api/samples/SMP-001/logs

# 6. 按批次 / 状态查询
curl -s "$B/api/samples?batch_no=BATCH-001"
curl -s "$B/api/samples?status=archived"
```

自动化验证：`cargo test` 运行 27 个集成测试，覆盖状态流转（闭环/跳跃/回退/非法值）、输入校验、批量导入（全部成功/部分重复回滚/批次不存在）、位置占用冲突与归档释放、位置变更日志（含旧/新位置）、位置查询（当前占用样本 + 迁移记录）、异常标记与解除、按批次/状态/位置的查询过滤，以及统一错误结构。
