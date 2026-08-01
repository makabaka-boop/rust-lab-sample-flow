# rust-lab-sample-flow

实验样本流转 API 服务，用于管理科研样本从采集、入库、处理、转移到归档的全过程。服务使用 Rust + axum + SQLite 实现，只提供 REST API，不包含 Web 前端或图形界面。

## 启动命令

```bash
cargo run
```

服务默认监听：

```text
http://0.0.0.0:18101
```

可选环境变量：

```bash
DATABASE_URL="sqlite:/path/to/sample_flow.db" cargo run
```

未设置 `DATABASE_URL` 时，默认在项目当前工作目录生成：

```text
sample_flow.db
```

程序启动时会自动创建 SQLite 数据库文件和全部数据表，不需要手动执行 SQL 文件。

## 验证命令

```bash
cargo test
cargo check
```

## 目录结构

```text
src/
  main.rs         程序入口，初始化数据库、路由和 18101 端口监听
  lib.rs          模块导出
  routes.rs       路由定义
  handlers.rs     HTTP 请求处理
  service.rs      业务逻辑、状态流转、输入规则
  db.rs           SQLite 初始化、事务和数据访问
  models.rs       资源模型、请求结构、状态枚举
  validation.rs   字段校验
  error.rs        统一错误结构和错误转换
  extractors.rs   统一 JSON 请求提取器
tests/
  api.rs          HTTP API 集成测试
```

## 核心资源

- 样本批次 `batches`：批次编号、项目名称、负责人、创建时间、备注。
- 单个样本 `samples`：样本编号、所属批次、样本类型、当前状态、存放位置、最近处理时间。
- 操作日志 `operation_logs`：操作者、动作、说明、源状态、目标状态、时间。
- 异常标记 `anomalies`：污染、标签缺失、温控异常等问题及解除信息。
- 存放位置 `locations`：区域、冰箱编号、层架、格位，并对四级位置做唯一约束。

## 状态流转规则

样本状态只能顺序向前流转：

```text
collected -> stored -> processing -> transferred -> archived
```

规则说明：

- 新登记样本固定为 `collected`。
- 每次状态更新只能进入当前状态的下一个状态。
- 禁止跳过关键状态，例如 `collected -> archived` 会返回 409。
- 已归档样本不能再变更存放位置。
- 样本从未设置位置时，进入 `stored` 必须提供位置。

## 统一错误结构

所有错误响应均为 JSON：

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "sample_number 不能为空",
    "details": {}
  }
}
```

常见错误码：

- `VALIDATION_ERROR`：输入校验失败。
- `INVALID_STATE_TRANSITION`：非法状态流转。
- `NOT_FOUND`：资源不存在。
- `CONFLICT`：唯一键冲突、重复操作或业务冲突。
- `BAD_REQUEST`：请求 JSON 格式错误。
- `INTERNAL_ERROR`：服务内部或数据库错误。

## 接口分组

### 健康检查

- `GET /health`

### 批次

- `POST /api/batches`：创建批次。
- `GET /api/batches`：查询批次列表。
- `GET /api/batches/{id}`：查看单个批次。

创建批次示例：

```bash
curl -X POST http://localhost:18101/api/batches \
  -H 'Content-Type: application/json' \
  -d '{
    "batch_number": "BATCH-202608-001",
    "project_name": "肿瘤标志物研究",
    "owner": "张老师",
    "remark": "第一批样本"
  }'
```

### 存放位置

- `POST /api/locations`：创建或复用位置。
- `GET /api/locations`：按区域、冰箱编号、层架、格位筛选位置。
- `GET /api/locations/{id}`：查看位置详情，返回当前存放样本以及最近 10 次迁入/迁出记录。

位置会按 `(area, fridge_number, shelf, slot)` 去重复用。

#### 位置占用规则

- 同一个区域、冰箱编号、层架、格位在同一时间只能存放一个**未归档**样本。
- 数据库层通过部分唯一索引 `idx_unique_active_location` 强制约束（仅对 `status != 'archived'` 且位置非空的样本生效）。
- 服务层会在样本登记、入库、批量导入、位置变更前进行占用校验，冲突时返回 `409 CONFLICT`，错误详情中包含占用该位置的样本编号。
- 样本归档（`archived`）时会自动将其 `current_location_id` 置空，释放位置，之后该格位可被新样本使用。
- 位置变更（`PUT /api/samples/{id}/location`）会写入操作日志，日志中包含 `from_location_id`（旧位置）和 `to_location_id`（新位置）。

位置详情响应示例：

```json
{
  "location": { "id": 1, "area": "A区", "fridge_number": "F-01", "shelf": "L-02", "slot": "G-03" },
  "current_sample": { "id": 1, "sample_number": "SAMPLE-0001", "status": "stored" },
  "recent_movements": [
    {
      "log_id": 12,
      "sample_id": 1,
      "sample_number": "SAMPLE-0001",
      "operator": "alice",
      "action": "change_location",
      "description": "转移到新格位",
      "direction": "in",
      "from_location": null,
      "to_location": { "id": 1, "area": "A区", "fridge_number": "F-01", "shelf": "L-02", "slot": "G-03" },
      "created_at": "2026-08-01T10:00:00Z"
    }
  ]
}
```

其中 `direction` 取值：

- `in`：样本迁入该位置。
- `out`：样本迁出该位置。
- `related`：其他与该位置相关的记录（如同位置更新）。

### 样本登记与查询

- `POST /api/batches/{id}/samples`：登记单个样本。
- `POST /api/batches/{id}/samples/bulk-import`：批量导入样本。
- `GET /api/batches/{id}/samples`：按批次查询样本。
- `GET /api/samples`：按批次、状态和位置筛选样本。
- `GET /api/samples/{id}`：查看单个样本。
- `GET /api/samples/{id}/flow`：查看样本完整流转日志、当前状态和异常记录。

样本查询支持参数：

- `batch_id`
- `status`
- `location_id`
- `area`
- `fridge_number`
- `shelf`
- `slot`

登记样本示例：

```bash
curl -X POST http://localhost:18101/api/batches/1/samples \
  -H 'Content-Type: application/json' \
  -d '{
    "sample_number": "SAMPLE-0001",
    "sample_type": "血液",
    "location": {
      "area": "A区",
      "fridge_number": "F-01",
      "shelf": "L-02",
      "slot": "G-03"
    },
    "operator": "alice",
    "description": "采集登记"
  }'
```

### 批量导入样本

- `POST /api/batches/{id}/samples/bulk-import`：一次请求向指定批次导入多条样本。

请求体结构：

```json
{
  "samples": [
    {
      "sample_number": "SAMPLE-0001",
      "sample_type": "血液",
      "location": {
        "area": "A区",
        "fridge_number": "F-01",
        "shelf": "L-02",
        "slot": "G-03"
      }
    },
    {
      "sample_number": "SAMPLE-0002",
      "sample_type": "组织"
    }
  ],
  "operator": "alice",
  "description": "批次批量导入"
}
```

批量导入规则：

- 一次请求至少包含一条样本。
- 每条样本必须填写 `sample_number` 和 `sample_type`。
- `location` 可选；一旦提供，其中 `area`、`fridge_number`、`shelf`、`slot` 均不能为空。
- 同一请求内样本编号不能重复。
- 任一编号与库中已有样本重复、任一位置非法或任一样本类型为空，整批回滚，不会出现部分成功。
- 导入成功的样本初始状态均为 `collected`。
- 导入完成后会为每个样本自动写入一条初始操作日志，动作（`action`）统一为 `created`，记录操作者、说明、目标状态和时间。
- 路径中的批次不存在时返回 `404 NOT_FOUND`。

批量导入示例：

```bash
curl -X POST http://localhost:18101/api/batches/1/samples/bulk-import \
  -H 'Content-Type: application/json' \
  -d '{
    "samples": [
      {"sample_number": "SAMPLE-0001", "sample_type": "血液"},
      {"sample_number": "SAMPLE-0002", "sample_type": "DNA"}
    ],
    "operator": "alice",
    "description": "批量导入"
  }'
```

### 样本状态与位置变更

- `PUT /api/samples/{id}/status`：更新样本状态。
- `PUT /api/samples/{id}/location`：变更样本存放位置。

状态更新示例：

```bash
curl -X PUT http://localhost:18101/api/samples/1/status \
  -H 'Content-Type: application/json' \
  -d '{
    "status": "stored",
    "operator": "alice",
    "description": "入库",
    "location": {
      "area": "A区",
      "fridge_number": "F-01",
      "shelf": "L-02",
      "slot": "G-03"
    }
  }'
```

### 异常标记

- `POST /api/samples/{id}/anomalies`：标记异常。
- `GET /api/samples/{id}/anomalies`：查看指定样本异常。
- `GET /api/anomalies`：查看异常列表，可按 `sample_id`、`resolved` 过滤。
- `POST /api/anomalies/{id}/resolve`：解除异常。

允许的异常类型：

- `contamination`
- `label_missing`
- `temperature_abnormal`
- `other`

## 验证样本流转闭环的方法

启动服务后，按以下顺序调用接口即可验证完整闭环：

1. 创建批次：`POST /api/batches`。
2. 登记样本：`POST /api/batches/{id}/samples`，此时状态为 `collected`。
3. 入库：`PUT /api/samples/{id}/status`，目标状态为 `stored`，必须带位置。
4. 处理：`PUT /api/samples/{id}/status`，目标状态为 `processing`。
5. 转移：`PUT /api/samples/{id}/status`，目标状态为 `transferred`。
6. 归档：`PUT /api/samples/{id}/status`，目标状态为 `archived`。
7. 查看轨迹：`GET /api/samples/{id}/flow`。

也可以直接运行：

```bash
cargo test completes_sample_flow_closure -- --nocapture
```

该测试会自动验证操作日志动作依次为：

```text
register -> store -> process -> transfer -> archive
```

状态跳步、缺失入库位置、重复批量导入、非法异常类型、归档后移动位置等行为均由测试覆盖。
