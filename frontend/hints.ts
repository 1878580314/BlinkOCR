/** Compact explanations shared by pointer and keyboard interaction. */
export const hints = {
  model:
    "决定模型所需的文件、字符集与推理方式。请选择与本地权重相同的模型系列。",
  weights:
    "识别使用的 GGUF 主模型。请选择与模型类型对应的文件；量化越低通常占用越少。",
  projector:
    "把图像转换为模型能读取的特征。必须与主模型配套；更换模型系列时同时更换此文件。",
  detection:
    "查找图中的文字区域。选择当前 PP-OCRv6 规格的检测 ONNX 文件，并配套加载识别模型。",
  recognition:
    "读取检测区域中的文字。选择与检测模型相同规格的识别 ONNX 文件；字符字典会按规格自动匹配。",
  backend:
    "运行模型的本地推理程序。通常保持内置路径；使用自行安装的兼容程序时再更改。",
  device:
    "自动模式优先使用可用 GPU，显存不足时混合分配或改用 CPU。希望减少 GPU 占用时选择 CPU。",
  context:
    "一次识别可使用的图像、提示词与输出 token 总量。通常保持 16384；大文档输出不完整时可增大，同时会增加内存和显存占用。",
  threads:
    "CPU 计算使用的线程数，默认取逻辑核心数的一半；CPU 满载或界面卡顿时下调，再比较识别耗时。",
  imageTokens:
    "图像处理使用的最小 token 预算。模型默认会按模型配置处理；小字识别不足时可提高，同时会增加耗时和内存占用。",
  port: "本地推理服务使用的端口。通常保持默认值；端口被其他程序占用时换一个未使用的端口。",
  timeout:
    "等待模型加载完成的最长时间。通常保持 180 秒；大模型或慢速磁盘加载超时时可提高。",
  mode: "选择当前模型提供的识别任务，按选区内容切换。",
  general:
    "提取图片中的文字与结构，表格和公式按当前模型的格式输出。适合混合内容文档。",
  text: "提取图中文字。适合截图、段落和连续整理；PP-OCRv6 与 Windows OCR 使用此模式。",
  table: "要求模型识别表格结构。适合表格选区。",
  formula: "要求视觉语言模型输出 LaTeX 公式。适合只包含公式的选区。",
  chart: "解析图表中的内容。适合统计图和科学图表。",
  spotting: "识别文字并返回位置。适合需要文字坐标的图片。",
  seal: "识别印章中的文字。请框选包含完整印章的区域。",
  code: "提取代码内容。请框选完整代码段。",
  layout:
    "提取文字、位置与元素类别，按阅读顺序输出 JSON。适合需要版面信息的文档。",
  output:
    "限制一次生成的 token 数。不限仍受上下文长度约束；通常保持不限，需要控制输出长度时再设置数值。",
  dimension:
    "识别前按比例缩小图片长边。细小文字较多时可提高此值；需要更快识别时下调。",
  hotkeys:
    "在其他程序中也能使用框选和退出快捷键。与其他软件冲突时修改按键或关闭此项。",
  captureKey:
    "点击后直接按下要使用的单键或组合键，Esc 取消录入。优先选择其他程序较少占用的按键。",
  quitKey: "点击后按下退出应用的组合键，Esc 取消录入。应与框选快捷键不同。",
  copy: "识别完成后自动将全文写入剪贴板。经常直接粘贴结果时开启，需要保留原剪贴板内容时关闭。",
  pin: "使窗口显示在其他程序上方。需要对照内容时开启；截图前会隐藏此窗口。",
  continuous:
    "将后续识别追加到当前结果，并保留手动修改。多图整理时开启；开始另一份内容时点击新建。",
  quantization:
    "量化降低权重体积与显存需求。空间充足可选 BF16/F16；优先节省显存可试 Q4_K_M 或 Q5_K_M。",
  onnxPrecision:
    "当前提供官方 FP32 检测与识别模型。Tiny 优先速度，Small 平衡速度与精度，Medium 优先精度。",
  language:
    "选择系统已安装的 OCR 语言，自动模式跟随 Windows 用户语言。需要更多语言时，在系统语言设置中安装文字识别组件。",
};

let sequence = 0;
let dismissActive: (() => void) | undefined;
export function hint(node: HTMLElement, content: string) {
  const tooltip = document.createElement("div");
  tooltip.id = `option-hint-${++sequence}`;
  tooltip.className = "option-tooltip";
  tooltip.setAttribute("role", "tooltip");
  tooltip.textContent = content;
  tooltip.hidden = true;
  document.body.append(tooltip);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let hovering = false;
  let dismissed = false;
  const described = new Set<HTMLElement>();
  function hide() {
    clearTimeout(timer);
    tooltip.hidden = true;
    if (dismissActive === hide) dismissActive = undefined;
  }
  function show() {
    if (dismissed) return;
    clearTimeout(timer);
    dismissActive?.();
    dismissActive = hide;
    const controls = node.matches("input,select,button")
      ? [node]
      : [...node.querySelectorAll<HTMLElement>("input,select,button")];
    for (const control of controls) {
      const values = new Set(
        (control.getAttribute("aria-describedby") ?? "")
          .split(/\s+/)
          .filter(Boolean),
      );
      values.add(tooltip.id);
      control.setAttribute("aria-describedby", [...values].join(" "));
      described.add(control);
    }
    tooltip.style.maxWidth = `${Math.min(320, innerWidth - 24)}px`;
    tooltip.hidden = false;
    const rect = node.getBoundingClientRect();
    const box = tooltip.getBoundingClientRect();
    tooltip.style.left = `${Math.max(12, Math.min(rect.left, innerWidth - box.width - 12))}px`;
    const below = rect.bottom + 8;
    tooltip.style.top = `${Math.max(12, below + box.height <= innerHeight - 12 ? below : rect.top - box.height - 8)}px`;
  }
  const enter = () => {
    hovering = true;
    dismissed = false;
    timer = setTimeout(show, 400);
  };
  const leave = () => {
    hovering = false;
    if (!node.contains(document.activeElement)) hide();
  };
  const blur = (event: FocusEvent) => {
    if (!hovering && !node.contains(event.relatedTarget as Node | null)) hide();
  };
  const escape = (event: KeyboardEvent) => {
    if (event.key === "Escape" && !tooltip.hidden) {
      event.preventDefault();
      event.stopImmediatePropagation();
      dismissed = true;
      hide();
    }
  };
  const moved = () => {
    if (!dismissed && node.contains(document.activeElement)) {
      const rect = node.getBoundingClientRect();
      if (rect.bottom > 0 && rect.top < innerHeight) { show(); return; }
    }
    hide();
    if (!dismissed && (hovering || node.contains(document.activeElement)))
      timer = setTimeout(() => {
        const rect = node.getBoundingClientRect();
        if (
          (node.matches(":hover") || node.contains(document.activeElement)) &&
          rect.bottom > 0 &&
          rect.top < innerHeight
        )
          show();
      }, 400);
  };
  const focus = () => {
    dismissed = false;
    show();
  };
  const click = () => {
    dismissed = true;
    hide();
  };
  node.addEventListener("pointerenter", enter);
  node.addEventListener("pointerleave", leave);
  node.addEventListener("focusin", focus);
  node.addEventListener("focusout", blur);
  node.addEventListener("click", click);
  window.addEventListener("keydown", escape, true);
  window.addEventListener("resize", moved);
  window.addEventListener("scroll", moved, true);
  return {
    update(value: string) {
      tooltip.textContent = value;
    },
    destroy() {
      hide();
      tooltip.remove();
      for (const control of described) {
        const ids = (control.getAttribute("aria-describedby") ?? "")
          .split(/\s+/)
          .filter((id) => id && id !== tooltip.id);
        if (ids.length) control.setAttribute("aria-describedby", ids.join(" "));
        else control.removeAttribute("aria-describedby");
      }
      node.removeEventListener("pointerenter", enter);
      node.removeEventListener("pointerleave", leave);
      node.removeEventListener("focusin", focus);
      node.removeEventListener("focusout", blur);
      node.removeEventListener("click", click);
      window.removeEventListener("keydown", escape, true);
      window.removeEventListener("resize", moved);
      window.removeEventListener("scroll", moved, true);
    },
  };
}
