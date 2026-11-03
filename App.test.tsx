// Drift GUI 的 headless 冒烟测试（`nsh test` 运行；`open(".")` 挂载 Rust 根组件）。
// 注意：核心的 redb 数据库同进程只能打开一次，因此全部断言共享一次 open()。
// 隔离数据目录：DRIFT_DATA_DIR=<临时目录> nsh test
// headless 判定为移动布局，文案按移动板断言。
import { expect, open, test } from "nsh/test";

test("空态 → 配对面板冒烟", async () => {
  const app = await open(".");
  const texts = () => app.texts();

  // 空状态（移动文案）
  expect(texts()).toContain("先配对一台设备");
  expect(texts()).toContain("01");
  expect(texts()).toContain("02");
  expect(texts()).toContain("03");

  // 无发送按钮 + 球体模型说明（v1.5：输入浮出球体，清空收起）
  expect(texts()).toContain("配对后，输入文本或添加文件即弹出设备层");
  expect(texts()).toContain("无发送按钮，点设备即发，改内容可增量补发");
  expect(texts()).toContain("清空输入即收起球体，回到会话");

  // 打开配对面板（移动端主按钮文案）
  await app.press("查找附近设备");
  expect(texts()).toContain("附近设备");
  expect(texts()).toContain("核对配对码");
  expect(texts()).toContain("3 分钟内有效");
  expect(texts()).toContain("从上面选一台设备，两端会同时出现同一组号码");
  expect(texts()).toContain("号码只用于两端对照，不会上传 · 配对后可随时撤销");
});
