# 媒体绑定实施计划（Media Binding）

> 复合媒体配对（Live Photo / RAW+JPEG / 连拍）的检测、配对与落库计划。
> 现状与优先级经维护者确认（2026-08-09）。

---

## 现状

- **检测**：`media/binding.rs::BindingDetector` 支持三类配对——
  Live Photo（HEIC/JPEG + MOV/MP4）、RAW+JPG（DNG/ARW/CR2/NEF… + JPEG/HEIF）、
  连拍序列（编号文件名）。
- **配对信号**：仅**同文件名**（file stem 相等）。不读文件内容。
- **未接线**：导入管线不调用 `find_bindings`；配对结果纯内存、算完即弃。
- **休眠存储**：`media_groups` 表 + `files.group_id` / `role` / `exif_fp`
  三列已建好，无任何读写者。
- 单元测试覆盖检测逻辑（`binding.rs` tests + E2E `test_binding.py`）。

## 已知局限

1. 同名即配对 → 巧合同名误绑（假阳性）；重命名后绑不上（假阴性）。
2. Live Photo 不校验 Apple ContentIdentifier。
3. 配对只在同目录 basename 层面工作。

---

## P1 — EXIF 指纹配对（RAW+JPEG）

**目标**：RAW+JPEG 配对增加 EXIF 信号，解决重命名假阴性。

**方案**：
- `exif_fp = H(DateTimeOriginal + SubSecTimeOriginal + BodySerialNumber)`
  （有 `ImageUniqueID` 时优先用它）。
- 导入时在**现有的一次 EXIF 读取**中顺带计算（`ops/exif.rs` 已为路径模板
  读取日期/设备，零额外 IO），落库到 `files.exif_fp`。
- 配对规则：同名 OR `exif_fp` 相同。`exif_fp` 落库后配对即查表。

**边界（测试必须锁定）**：
- 无亚秒时钟的机身同秒连拍会撞指纹 → 需同名/序号兜底，或拒绝自动配对并报告。
- 后期软件导出的 JPEG 可能重写 EXIF → 指纹漂移，退回同名规则。
- kamadak-exif 对 DNG/CR2/NEF/ARW 的可读性需固件验证（E2E 用 exiftool 写
  真实 EXIF，不用 Python EXIF 库——见测试 skill）。

## P2 — Live Photo ContentIdentifier 校验（必要，低优先级，暂缓）

**目标**：用 Apple 语义校验 Live Photo 配对，消除同名假阳性/假阴性。

**方案**：读取并比对两侧 UUID——HEIC 侧 Apple MakerNote 的
`ContentIdentifier`，MOV 侧 `com.apple.quicktime.content.identifier`
元数据键。一致才判 Live Photo；`exif_fp` 作辅助证据。

**暂缓原因**（维护者 2026-08-09）：必要但优先级低；MakerNote/QuickTime
元数据解析工作量相对集中，排在 P1 与接线之后。

## P3 — 接线与落库

- import 时运行绑定检测 → 写 `media_groups`（group_type /
  content_identifier / captured_at）+ 回填 `files.group_id` / `role`
  （primary/motion/auxiliary）。
- clone 分组过滤维度（相机/分组）在绑定落库后才有意义
  （ARCHITECTURE.md §6.1 已登记此依赖）。
- 事务纪律：绑定写入并入 Stage E 整批单事务（failure-handling.md G5）。

## 测试要求

- 固件：exiftool 写入 DateTimeOriginal/SubSecTime/BodySerialNumber/
  ContentIdentifier（与真实相机文件一致）。
- 用例：同机同时刻配对、重命名配对（exif_fp 命中）、连拍同秒冲突、
  异机同时刻不配对（序列号区分）、EXIF 被重写后退回同名规则、
  Live Photo UUID 一致/不一致（P2 落地时）。
