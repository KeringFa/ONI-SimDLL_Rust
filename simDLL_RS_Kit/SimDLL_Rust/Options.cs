using HarmonyLib;
using System;
using System.Collections;
using System.IO;
using System.Reflection;
using TMPro;
using UnityEngine;
using UnityEngine.UI;

namespace SimDLL_Rust
{
    /// <summary>
    /// mod 配置入口：ModsScreen 条目上的 Options 按钮 + InfoDialogScreen 配置屏
    /// + 翻译注册。UI 模式参考 Optimize_Steam_Turbine（用户其他 mod）。
    /// 配置项：启用日志（复选框）。OK 后仅保存 config.json 并弹出"需重启"提示
    /// （套用 Optimize_Steam_Turbine 的 ConfirmDialogScreen 重启提示）；
    /// 配置在下次启动 OnLoad 时经 ModConfig.Apply() 生效——日志只在下次启动后生成。
    /// 日志级别输入已屏蔽（普通玩家用不到；发布前再讨论哪些日志应被输出）。
    /// </summary>
    public static class Options
    {
        public const string STATIC_ID = "SimDLL_Rust";
        private const string OPTIONS_BUTTON_NAME = "SimDLL_Rust_OptionsButton";
        private static ModsScreen _modsScreen;

        /// <summary>
        /// Patch ModsScreen.BuildDisplay：为本 mod 条目添加 Options 按钮。
        /// </summary>
        [HarmonyPatch(typeof(ModsScreen), "BuildDisplay")]
        public class ModsScreenBuildDisplayPatch
        {
            public static void Postfix(ModsScreen __instance, object ___displayedMods)
            {
                if (___displayedMods == null) return;
                _modsScreen = __instance;

                var mods = Global.Instance.modManager.mods;
                foreach (var entry in (IEnumerable)___displayedMods)
                {
                    var entryTraverse = Traverse.Create(entry);
                    int modIndex = entryTraverse.Field<int>("mod_index").Value;
                    var rectTransform = entryTraverse.Field<RectTransform>("rect_transform").Value;

                    if (modIndex < 0 || modIndex >= mods.Count || rectTransform == null) continue;
                    if (mods[modIndex].staticID != STATIC_ID) continue;

                    if (rectTransform.TryGetComponent(out HierarchyReferences references))
                    {
                        var manageButton = references.GetReference<KButton>("ManageButton");
                        if (manageButton == null) break;
                        if (manageButton.transform.parent.Find(OPTIONS_BUTTON_NAME) != null) break;

                        var optionsButton = Util.KInstantiateUI<KButton>(
                            manageButton.gameObject, manageButton.transform.parent.gameObject, true);
                        optionsButton.name = OPTIONS_BUTTON_NAME;
                        optionsButton.transform.SetSiblingIndex(manageButton.transform.GetSiblingIndex() - 1);

                        var locText = optionsButton.GetComponentInChildren<LocText>();
                        if (locText != null)
                        {
                            locText.text = global::STRINGS.UI.FRONTEND.PAUSE_SCREEN.OPTIONS;
                        }
                        optionsButton.onClick += ShowOptionsDialog;
                    }
                    break;
                }
            }
        }

        /// <summary>
        /// 弹出 Options 对话框：启用日志复选框 + 日志级别输入框。
        /// 使用游戏原生 InfoDialogScreen（KModalScreen，主菜单安全）。
        /// </summary>
        private static void ShowOptionsDialog()
        {
            var screenPrefabs = ScreenPrefabs.Instance;
            if (screenPrefabs == null || screenPrefabs.InfoDialogScreen == null)
            {
                Debug.LogError("[SimDLL_Rust] ScreenPrefabs 或 InfoDialogScreen 不可用");
                return;
            }

            Transform parent = _modsScreen != null
                ? _modsScreen.transform
                : GameScreenManager.Instance?.transform;
            if (parent == null)
            {
                Debug.LogError("[SimDLL_Rust] 无法获取 UI parent");
                return;
            }

            var dialog = Util.KInstantiateUI<InfoDialogScreen>(
                screenPrefabs.InfoDialogScreen.gameObject, parent.gameObject, false);
            ((KModalScreen)dialog).pause = false;

            bool currentEnabled = ModConfig.LogEnabled;
            bool newEnabled = currentEnabled;
            bool currentHt = ModConfig.HyperThreadOptimizationEnabled;
            bool newHt = currentHt;
            bool htAvailable = CpuInfo.HasHyperThreading();
            int currentReservedCores = ModConfig.ReservedCores;
            int newReservedCores = currentReservedCores;
            bool currentSlice = ModConfig.SliceEnabled;
            bool newSlice = currentSlice;
            int currentSliceSize = ModConfig.SliceSize;
            int newSliceSize = currentSliceSize;

            dialog.SetHeader(STRINGS.UI.FRONTEND.MOD_OPTIONS.DIALOG_TITLE);

            // 设置面板（圆角背景 + VerticalLayoutGroup），样式参考游戏原生 SAVE 子窗口
            var contentContainer = Traverse.Create(dialog).Field<GameObject>("contentContainer").Value;
            if (contentContainer == null)
            {
                Debug.LogError("[SimDLL_Rust] InfoDialogScreen.contentContainer 不可用");
                UnityEngine.Object.Destroy(dialog.gameObject);
                return;
            }
            var panelObj = new GameObject("SettingsPanel", typeof(RectTransform));
            panelObj.transform.SetParent(contentContainer.transform, false);
            var bg = panelObj.AddComponent<Image>();
            bg.color = new Color(0.35f, 0.35f, 0.35f, 0.8f);
            bg.type = Image.Type.Sliced;
            var panelLayout = panelObj.AddComponent<VerticalLayoutGroup>();
            panelLayout.spacing = 8f;
            panelLayout.childControlWidth = true;
            panelLayout.childControlHeight = true;
            panelLayout.childForceExpandWidth = true;
            panelLayout.childForceExpandHeight = false;
            panelLayout.padding = new RectOffset(15, 15, 10, 10);
            var panelLE = panelObj.AddComponent<LayoutElement>();
            panelLE.flexibleWidth = 1;

            var plainTextPrefab = dialog.GetPlainTextPrefab();

            // ===== ToggleRow：启用日志 =====
            var toggleRow = NewRow(panelObj, "ToggleRow", 24);
            if (plainTextPrefab != null)
            {
                var labelObj = Util.KInstantiateUI(plainTextPrefab.gameObject, toggleRow, true);
                var csf = labelObj.GetComponent<ContentSizeFitter>();
                if (csf != null) csf.enabled = false;
                var labelText = labelObj.GetComponentInChildren<LocText>();
                if (labelText != null)
                {
                    labelText.text = STRINGS.UI.FRONTEND.MOD_OPTIONS.LOG_ENABLED_LABEL;
                    labelText.alignment = TextAlignmentOptions.Left;
                    labelText.raycastTarget = false;
                }
                var labelLE = labelObj.GetComponent<LayoutElement>() ?? labelObj.AddComponent<LayoutElement>();
                labelLE.preferredWidth = 300;
                labelLE.flexibleWidth = 1;
            }
            MultiToggle toggle = null;
            GameObject checkboxPrefab = null;
            try { checkboxPrefab = Assets.UIPrefabs.TableScreenWidgets.Checkbox; }
            catch { checkboxPrefab = null; }
            if (checkboxPrefab != null)
            {
                toggle = Util.KInstantiateUI<MultiToggle>(checkboxPrefab, toggleRow, true);

                var le = toggle.gameObject.GetComponent<LayoutElement>()
                    ?? toggle.gameObject.AddComponent<LayoutElement>();
                le.preferredWidth = 20;
                le.preferredHeight = 20;
                le.minWidth = 20;
                le.minHeight = 20;

                // 修正 toggle_image 与 ToggleState（参照 Optimize_Steam_Turbine 验证过的写法）：
                // Checkbox prefab 的 toggle_image 默认带 rect_margins 偏移，不做处理会导致
                // 勾选图标/点击区域错位（表现为"无法勾选"）。置中锚点 + 固定 16x16 + 禁用
                // use_rect_margins 后交互恢复正常。
                var toggleImage = Traverse.Create(toggle).Field<Image>("toggle_image").Value;
                var states = toggle.states;
                var newStates = new ToggleState[states.Length];
                for (int i = 0; i < states.Length; i++)
                {
                    newStates[i] = states[i];
                    newStates[i].use_rect_margins = false;
                }
                toggle.states = newStates;

                if (toggleImage != null)
                {
                    var tiRect = toggleImage.GetComponent<RectTransform>();
                    tiRect.anchorMin = new Vector2(0.5f, 0.5f);
                    tiRect.anchorMax = new Vector2(0.5f, 0.5f);
                    tiRect.pivot = new Vector2(0.5f, 0.5f);
                    tiRect.anchoredPosition = Vector2.zero;
                    tiRect.sizeDelta = new Vector2(16, 16);
                }

                toggle.onClick = () =>
                {
                    newEnabled = !newEnabled;
                    toggle.ChangeState(newEnabled ? 1 : 0);
                };
                toggle.ChangeState(newEnabled ? 1 : 0);
            }

            // ===== ToggleRow2：启用超线程优化（方向 1）=====
            var toggleRow2 = NewRow(panelObj, "ToggleRow2", 24);
            if (plainTextPrefab != null)
            {
                var labelObj = Util.KInstantiateUI(plainTextPrefab.gameObject, toggleRow2, true);
                var csf = labelObj.GetComponent<ContentSizeFitter>();
                if (csf != null) csf.enabled = false;
                var labelText = labelObj.GetComponentInChildren<LocText>();
                if (labelText != null)
                {
                    labelText.text = STRINGS.UI.FRONTEND.MOD_OPTIONS.HYPERTHREAD_LABEL;
                    labelText.alignment = TextAlignmentOptions.Left;
                    labelText.raycastTarget = false;
                }
                var labelLE = labelObj.GetComponent<LayoutElement>() ?? labelObj.AddComponent<LayoutElement>();
                labelLE.preferredWidth = 300;
                labelLE.flexibleWidth = 1;
            }
            MultiToggle htToggle = null;
            GameObject htCheckboxPrefab = null;
            try { htCheckboxPrefab = Assets.UIPrefabs.TableScreenWidgets.Checkbox; }
            catch { htCheckboxPrefab = null; }
            if (htCheckboxPrefab != null)
            {
                htToggle = Util.KInstantiateUI<MultiToggle>(htCheckboxPrefab, toggleRow2, true);
                var le = htToggle.gameObject.GetComponent<LayoutElement>()
                    ?? htToggle.gameObject.AddComponent<LayoutElement>();
                le.preferredWidth = 20;
                le.preferredHeight = 20;
                le.minWidth = 20;
                le.minHeight = 20;

                var htToggleImage = Traverse.Create(htToggle).Field<Image>("toggle_image").Value;
                var htStates = htToggle.states;
                var htNewStates = new ToggleState[htStates.Length];
                for (int i = 0; i < htStates.Length; i++)
                {
                    htNewStates[i] = htStates[i];
                    htNewStates[i].use_rect_margins = false;
                }
                htToggle.states = htNewStates;
                if (htToggleImage != null)
                {
                    var tiRect = htToggleImage.GetComponent<RectTransform>();
                    tiRect.anchorMin = new Vector2(0.5f, 0.5f);
                    tiRect.anchorMax = new Vector2(0.5f, 0.5f);
                    tiRect.pivot = new Vector2(0.5f, 0.5f);
                    tiRect.anchoredPosition = Vector2.zero;
                    tiRect.sizeDelta = new Vector2(16, 16);
                }
                htToggle.onClick = () =>
                {
                    if (!htAvailable) return;
                    newHt = !newHt;
                    htToggle.ChangeState(newHt ? 1 : 0);
                };
                htToggle.ChangeState(newHt ? 1 : 0);
                if (!htAvailable)
                {
                    // 无超线程：灰显禁用（不可交互 + 图标降透明度）
                    htToggle.enabled = false;
                    if (htToggleImage != null)
                    {
                        var c = htToggleImage.color;
                        c.a = 0.4f;
                        htToggleImage.color = c;
                    }
                }
            }

            // ===== InputRow：预留物理线程数（1~4，默认 1）=====
            var reservedRowObj = NewRow(panelObj, "ReservedCoresRow", 24);
            if (plainTextPrefab != null)
            {
                var reservedLabelObj = Util.KInstantiateUI(plainTextPrefab.gameObject, reservedRowObj, true);
                var reservedCsf = reservedLabelObj.GetComponent<ContentSizeFitter>();
                if (reservedCsf != null) reservedCsf.enabled = false;
                var reservedLabelText = reservedLabelObj.GetComponentInChildren<LocText>();
                if (reservedLabelText != null)
                {
                    reservedLabelText.text = STRINGS.UI.FRONTEND.MOD_OPTIONS.RESERVED_CORES_LABEL;
                    reservedLabelText.alignment = TextAlignmentOptions.Left;
                    reservedLabelText.raycastTarget = false;
                }
                var reservedLabelLE = reservedLabelObj.GetComponent<LayoutElement>()
                    ?? reservedLabelObj.AddComponent<LayoutElement>();
                reservedLabelLE.preferredWidth = 300;
                reservedLabelLE.flexibleWidth = 1;
            }

            // 输入框：克隆 RailModUploadMenu 的 modVersion（TMP_InputField），
            // IntegerNumber + 空值回退默认 + 钳制 1~4（套用 Optimize_Steam_Turbine 写法）。
            TMP_InputField reservedInput = null;
            var railMenu = screenPrefabs.RailModUploadMenu;
            if (railMenu != null)
            {
                var versionSource = Traverse.Create(railMenu).Field<TMP_InputField>("modVersion").Value;
                if (versionSource != null)
                {
                    var inputClone = Util.KInstantiateUI(versionSource.gameObject, reservedRowObj, true);

                    var inputLe = inputClone.GetComponent<LayoutElement>();
                    if (inputLe == null) inputLe = inputClone.AddComponent<LayoutElement>();
                    inputLe.preferredWidth = 80;
                    inputLe.minWidth = 60;

                    reservedInput = inputClone.GetComponent<TMP_InputField>();
                    if (reservedInput != null)
                    {
                        reservedInput.contentType = TMP_InputField.ContentType.IntegerNumber;
                        reservedInput.characterLimit = 1;
                        reservedInput.text = newReservedCores.ToString();

                        var placeholder = reservedInput.placeholder as TextMeshProUGUI;
                        if (placeholder != null)
                            placeholder.text = ModConfig.DEFAULT_RESERVED_CORES.ToString();

                        reservedInput.onEndEdit.AddListener(val =>
                        {
                            if (string.IsNullOrEmpty(val))
                            {
                                newReservedCores = ModConfig.DEFAULT_RESERVED_CORES;
                                reservedInput.text = newReservedCores.ToString();
                            }
                            else if (int.TryParse(val, out int result))
                            {
                                newReservedCores = ModConfig.ClampReservedCores(result);
                                reservedInput.text = newReservedCores.ToString();
                            }
                        });
                    }
                }
                else
                {
                    Debug.LogWarning("[SimDLL_Rust] modVersion 不可用，预留线程数输入框不可用");
                }
            }
            else
            {
                Debug.LogWarning("[SimDLL_Rust] RailModUploadMenu 不可用，预留线程数输入框不可用");
            }

            // ===== Row：温度切片（OFF / 尺寸）—— 单一下拉同时表达"开关"与"尺寸" =====
            // 用户定稿的交互：下拉共两类选项，最顶部 OFF（默认项，即关闭切片），
            // 其下 64×64 / 32×32 / 16×16 / 8×8 —— 玩家选任一尺寸即启用切片。
            // 因此不再单独放"启用切片"复选框（两者合并，避免开关与尺寸状态脱节）。
            // 克隆的是游戏 GraphicsOptionsScreen（主菜单"图形"设置）里 Resolution 用的
            // 原生 Dropdown —— 游戏自带的固定式下拉，兼容性最好。
            var sliceSizeRowObj = NewRow(panelObj, "SliceSizeRow", 28);
            if (plainTextPrefab != null)
            {
                var sliceSizeLabelObj = Util.KInstantiateUI(plainTextPrefab.gameObject, sliceSizeRowObj, true);
                var sliceSizeCsf = sliceSizeLabelObj.GetComponent<ContentSizeFitter>();
                if (sliceSizeCsf != null) sliceSizeCsf.enabled = false;
                var sliceSizeLabelText = sliceSizeLabelObj.GetComponentInChildren<LocText>();
                if (sliceSizeLabelText != null)
                {
                    sliceSizeLabelText.text = STRINGS.UI.FRONTEND.MOD_OPTIONS.SLICE_SIZE_LABEL;
                    sliceSizeLabelText.alignment = TextAlignmentOptions.Left;
                    sliceSizeLabelText.raycastTarget = false;
                }
                var sliceSizeLabelLE = sliceSizeLabelObj.GetComponent<LayoutElement>()
                    ?? sliceSizeLabelObj.AddComponent<LayoutElement>();
                sliceSizeLabelLE.preferredWidth = 300;
                sliceSizeLabelLE.flexibleWidth = 1;
            }
            // 克隆下拉：成功则用下拉选尺寸；失败则回退 TMP 输入框（见下行注释）。
            Dropdown sliceSizeDropdown = CloneResolutionDropdown(sliceSizeRowObj.transform, out GameObject sliceDropdownGO);
            TMP_InputField sliceSizeInput = null;
            if (sliceSizeDropdown != null)
            {
                // —— 用克隆的游戏下拉：index 0 = OFF，1..4 = 尺寸（由大到小）——
                // 选项文案纯文本（Unity Dropdown 不走 LocText）。
                // 选 OFF → 关闭切片（保留上次尺寸，方便重新开启）；选任一尺寸 → 启用。
                int[] sliceChoices = { 64, 32, 16, 8 };
                sliceSizeDropdown.ClearOptions();
                var opts = new System.Collections.Generic.List<Dropdown.OptionData>();
                opts.Add(new Dropdown.OptionData("OFF"));
                for (int ci = 0; ci < sliceChoices.Length; ci++)
                {
                    // 用 ASCII 'x' 而非乘号 '×'（U+00D7）：游戏字体不保证含乘号字形，
                    // 显示成方块/问号会误导玩家；游戏自带分辨率选项也用 "1920x1080" 写法。
                    string label = sliceChoices[ci] + "x" + sliceChoices[ci];
                    opts.Add(new Dropdown.OptionData(label));
                }
                sliceSizeDropdown.options = opts;

                // 回显：关闭 → 选 OFF(0)；启用 → 选对应尺寸，尺寸不在候选时钳到最近合法项
                int curIdx = 0;
                if (newSlice)
                {
                    int si = Array.IndexOf(sliceChoices, newSliceSize);
                    if (si < 0)
                    {
                        int nearest = ModConfig.DEFAULT_SLICE_SIZE;
                        int best = 0;
                        for (int ci = 0; ci < sliceChoices.Length; ci++)
                        {
                            if (Math.Abs(sliceChoices[ci] - newSliceSize) < Math.Abs(nearest - newSliceSize))
                            {
                                nearest = sliceChoices[ci];
                                best = ci;
                            }
                        }
                        newSliceSize = nearest;
                        si = best;
                    }
                    curIdx = si + 1; // +1 跳过 OFF
                }
                sliceSizeDropdown.value = curIdx;
                sliceSizeDropdown.onValueChanged.AddListener(idx =>
                {
                    if (idx <= 0)
                    {
                        // OFF：只关开关，尺寸保持上次值不变
                        newSlice = false;
                    }
                    else
                    {
                        newSlice = true;
                        newSliceSize = sliceChoices[idx - 1];
                    }
                });
            }
            else
            {
                // —— 回退：RailModUploadMenu.modVersion 输入框 ——
                Debug.LogWarning("[SimDLL_Rust] 分辨率下拉克隆失败，回退输入框");
                if (railMenu != null)
                {
                    var versionSource = Traverse.Create(railMenu).Field<TMP_InputField>("modVersion").Value;
                    if (versionSource != null)
                    {
                        var inputClone = Util.KInstantiateUI(versionSource.gameObject, sliceSizeRowObj, true);
                        var inputLe = inputClone.GetComponent<LayoutElement>();
                        if (inputLe == null) inputLe = inputClone.AddComponent<LayoutElement>();
                        inputLe.preferredWidth = 80;
                        inputLe.minWidth = 60;

                        sliceSizeInput = inputClone.GetComponent<TMP_InputField>();
                        if (sliceSizeInput != null)
                        {
                            sliceSizeInput.contentType = TMP_InputField.ContentType.IntegerNumber;
                            sliceSizeInput.characterLimit = 2;
                            sliceSizeInput.text = newSliceSize.ToString();

                            var placeholder = sliceSizeInput.placeholder as TextMeshProUGUI;
                            if (placeholder != null)
                                placeholder.text = ModConfig.DEFAULT_SLICE_SIZE.ToString();

                            sliceSizeInput.onEndEdit.AddListener(val =>
                            {
                                if (string.IsNullOrEmpty(val))
                                {
                                    newSliceSize = ModConfig.DEFAULT_SLICE_SIZE;
                                    sliceSizeInput.text = newSliceSize.ToString();
                                }
                                else if (int.TryParse(val, out int result))
                                {
                                    newSliceSize = ModConfig.ClampSliceSize(result);
                                    sliceSizeInput.text = newSliceSize.ToString();
                                }
                            });
                        }
                    }
                }
            }

            // ===== 下划线 + 提示 =====
            var dividerObj = new GameObject("Divider", typeof(RectTransform));
            dividerObj.transform.SetParent(panelObj.transform, false);
            var dividerImage = dividerObj.AddComponent<Image>();
            dividerImage.color = new Color(0.5f, 0.5f, 0.5f, 0.6f);
            var dividerLE = dividerObj.AddComponent<LayoutElement>();
            dividerLE.preferredHeight = 1;
            dividerLE.flexibleWidth = 1;

            // 提示1（白色）：日志默认保存位置
            if (plainTextPrefab != null)
            {
                var hintObj = Util.KInstantiateUI(plainTextPrefab.gameObject, panelObj, true);
                var hintCsf = hintObj.GetComponent<ContentSizeFitter>();
                if (hintCsf != null) hintCsf.enabled = false;
                var hintText = hintObj.GetComponentInChildren<LocText>();
                if (hintText != null)
                {
                    hintText.alignment = TextAlignmentOptions.Left;
                    hintText.text = STRINGS.UI.FRONTEND.MOD_OPTIONS.LOG_PATH_HINT;
                }
                var hintLE = hintObj.GetComponent<LayoutElement>() ?? hintObj.AddComponent<LayoutElement>();
                hintLE.flexibleWidth = 1;
            }

            // 提示2（白色）：超线程优化说明
            if (plainTextPrefab != null)
            {
                var htHintObj = Util.KInstantiateUI(plainTextPrefab.gameObject, panelObj, true);
                var htHintCsf = htHintObj.GetComponent<ContentSizeFitter>();
                if (htHintCsf != null) htHintCsf.enabled = false;
                var htHintText = htHintObj.GetComponentInChildren<LocText>();
                if (htHintText != null)
                {
                    htHintText.alignment = TextAlignmentOptions.Left;
                    htHintText.text = STRINGS.UI.FRONTEND.MOD_OPTIONS.HYPERTHREAD_HINT;
                }
                var htHintLE = htHintObj.GetComponent<LayoutElement>() ?? htHintObj.AddComponent<LayoutElement>();
                htHintLE.flexibleWidth = 1;
            }

            // 提示3（白色）：D4 温度切片说明
            if (plainTextPrefab != null)
            {
                var sliceHintObj = Util.KInstantiateUI(plainTextPrefab.gameObject, panelObj, true);
                var sliceHintCsf = sliceHintObj.GetComponent<ContentSizeFitter>();
                if (sliceHintCsf != null) sliceHintCsf.enabled = false;
                var sliceHintText = sliceHintObj.GetComponentInChildren<LocText>();
                if (sliceHintText != null)
                {
                    sliceHintText.alignment = TextAlignmentOptions.Left;
                    sliceHintText.text = STRINGS.UI.FRONTEND.MOD_OPTIONS.SLICE_HINT;
                }
                var sliceHintLE = sliceHintObj.GetComponent<LayoutElement>() ?? sliceHintObj.AddComponent<LayoutElement>();
                sliceHintLE.flexibleWidth = 1;
            }

            // 提示4（红色）：修改设置后需重启游戏生效
            if (plainTextPrefab != null)
            {
                var restartObj = Util.KInstantiateUI(plainTextPrefab.gameObject, panelObj, true);
                var restartCsf = restartObj.GetComponent<ContentSizeFitter>();
                if (restartCsf != null) restartCsf.enabled = false;
                var restartText = restartObj.GetComponentInChildren<LocText>();
                if (restartText != null)
                {
                    restartText.richText = true;
                    restartText.enableVertexGradient = false;
                    restartText.alignment = TextAlignmentOptions.Left;
                    restartText.text = "<color=#FF5050>" + STRINGS.UI.FRONTEND.MOD_OPTIONS.RESTART_HINT + "</color>";
                }
                var restartLE = restartObj.GetComponent<LayoutElement>() ?? restartObj.AddComponent<LayoutElement>();
                restartLE.flexibleWidth = 1;
            }

            // ===== OK / CANCEL =====
            dialog.AddOption(global::STRINGS.UI.CONFIRMDIALOG.OK, d =>
            {
                bool changed = newEnabled != currentEnabled || newHt != currentHt
                    || newReservedCores != currentReservedCores
                    || newSlice != currentSlice || newSliceSize != currentSliceSize;
                if (newEnabled != currentEnabled) ModConfig.SetLogEnabled(newEnabled);
                if (newHt != currentHt) ModConfig.SetHyperThreadOptimizationEnabled(newHt);
                if (newReservedCores != currentReservedCores) ModConfig.SetReservedCores(newReservedCores);
                if (newSlice != currentSlice) ModConfig.SetSliceEnabled(newSlice);
                if (newSliceSize != currentSliceSize) ModConfig.SetSliceSize(newSliceSize);
                if (changed) ModConfig.Save();
                d.Deactivate();
                if (changed)
                {
                    // 配置已保存；重启后 OnLoad 的 ModConfig.Apply() 才实际应用
                    // （RS_SetLogEnabled）——日志只在下次启动后生成。
                    ShowRestartConfirmDialog(parent);
                }
            }, true);
            dialog.AddDefaultCancel();

            dialog.Activate();
        }

        /// <summary>
        /// 弹出游戏原生 ConfirmDialogScreen，提示需要重启游戏（套用
        /// Optimize_Steam_Turbine 的完整屏幕模块）。完全复用原生
        /// "MODS CHANGED" 窗口的文本资源（游戏自带翻译）：
        /// 标题 MODS CHANGED / 消息 / RESTART / CONTINUE。
        /// </summary>
        private static void ShowRestartConfirmDialog(Transform parent)
        {
            var screenPrefabs = ScreenPrefabs.Instance;
            if (screenPrefabs == null || screenPrefabs.ConfirmDialogScreen == null) return;

            var confirmDialog = Util.KInstantiateUI<ConfirmDialogScreen>(
                screenPrefabs.ConfirmDialogScreen.gameObject, parent.gameObject, true);

            // 用空字符串格式化剔除 {0}（mod 列表），TrimStart() 移除前导换行
            string message = string.Format(
                global::STRINGS.UI.FRONTEND.MOD_DIALOGS.MODS_SCREEN_CHANGES.MESSAGE, "").TrimStart();

            confirmDialog.PopupConfirmDialog(
                message,
                () => { App.instance.Restart(); },
                () => { },
                null, null,
                global::STRINGS.UI.FRONTEND.MOD_DIALOGS.MODS_SCREEN_CHANGES.TITLE,
                global::STRINGS.UI.FRONTEND.MOD_DIALOGS.RESTART.OK,
                global::STRINGS.UI.FRONTEND.MOD_DIALOGS.RESTART.CANCEL,
                null
            );
        }

        private static GameObject NewRow(GameObject parent, string name, float preferredHeight)
        {
            var rowObj = new GameObject(name, typeof(RectTransform));
            rowObj.transform.SetParent(parent.transform, false);
            var layout = rowObj.AddComponent<HorizontalLayoutGroup>();
            layout.childAlignment = TextAnchor.MiddleLeft;
            layout.spacing = 10f;
            layout.childControlWidth = true;
            layout.childControlHeight = true;
            layout.childForceExpandWidth = false;
            layout.childForceExpandHeight = false;
            var le = rowObj.AddComponent<LayoutElement>();
            le.preferredHeight = preferredHeight;
            le.flexibleWidth = 1;
            return rowObj;
        }

        /// <summary>
        /// 克隆游戏主菜单"图形"设置（GraphicsOptionsScreen）里分辨率下拉（UnityEngine.UI.Dropdown）
        /// 到 `parent`，返回克隆的 Dropdown（克隆失败返回 null，调用方应回退其他控件）。
        ///
        /// 获取链路（全部为序列化引用，ScreenPrefabs 常驻单例可达，无需某屏已打开）：
        ///   ScreenPrefabs.Instance.OptionsScreen（OptionsMenuScreen prefab）
        ///     → 私有字段 graphicsOptionsScreenPrefab（GraphicsOptionsScreen prefab，Traverse 反射）
        ///     → 层级内找 Dropdown（路径 Find 优先，递归 GetComponentInChildren 兜底）
        ///
        /// 缩小宽度：原分辨率下拉较宽（GraphicsOptionsScreen 整行），此处把克隆体的
        /// LayoutElement 收到与输入框一致的宽度（preferredWidth 80 / minWidth 60）。
        /// </summary>
        private static Dropdown CloneResolutionDropdown(Transform parent, out GameObject clonedGO)
        {
            clonedGO = null;
            try
            {
                var screenPrefabs = ScreenPrefabs.Instance;
                if (screenPrefabs == null || screenPrefabs.OptionsScreen == null)
                {
                    Debug.LogWarning("[SimDLL_Rust] ScreenPrefabs.OptionsScreen 不可用，无法克隆下拉");
                    return null;
                }

                // 1) OptionsMenuScreen prefab → GraphicsOptionsScreen prefab（私有序列化字段）。
                //    用非泛型 GetValue() 取 object 再 cast——字段类型是 GraphicsOptionsScreen，
                //    泛型强转不匹配时 Traverse 会静默返回 null，难排查。
                var graphicsObj = Traverse.Create(screenPrefabs.OptionsScreen)
                    .Field("graphicsOptionsScreenPrefab").GetValue();
                var graphicsPrefab = graphicsObj as Component;
                if (graphicsPrefab == null)
                {
                    Debug.LogWarning("[SimDLL_Rust] graphicsOptionsScreenPrefab 不可用，无法克隆下拉");
                    return null;
                }

                // 2) 找 Resolution 下拉：优先按用户给出的层级路径（AssetStudio 解包路径），
                //    找不到再递归 GetComponentInChildren<Dropdown>（兜底拿第一个下拉）。
                //    注意 GraphicsOptionsScreen 里还有一个"颜色模式"下拉，路径定位更精确。
                Dropdown sourceDropdown = null;
                var resPath = graphicsPrefab.transform.Find("Content/DisplaySettings/Resolution/Resolution");
                if (resPath != null)
                {
                    sourceDropdown = resPath.GetComponent<Dropdown>();
                }
                if (sourceDropdown == null)
                {
                    sourceDropdown = graphicsPrefab.GetComponentInChildren<Dropdown>(true);
                }
                if (sourceDropdown == null)
                {
                    Debug.LogWarning("[SimDLL_Rust] GraphicsOptionsScreen 内找不到 Dropdown，无法克隆");
                    return null;
                }

                // 3) 克隆到 parent（KInstantiateUI 深拷贝全部子物体：caption/template/option 等）
                clonedGO = Util.KInstantiateUI(sourceDropdown.gameObject, parent.gameObject, true);
                var cloned = clonedGO.GetComponent<Dropdown>();
                if (cloned == null)
                {
                    Debug.LogWarning("[SimDLL_Rust] 克隆体缺少 Dropdown 组件");
                    return null;
                }

                // 4) 收窄到与输入框一致的宽度。分辨率下拉宽是因为其 caption 文本长；
                //    收窄 LayoutElement + 让 RectTransform 不再横向撑满。
                var le = clonedGO.GetComponent<LayoutElement>();
                if (le == null) le = clonedGO.AddComponent<LayoutElement>();
                le.preferredWidth = 80;
                le.minWidth = 60;
                le.flexibleWidth = 0;
                var rt = clonedGO.GetComponent<RectTransform>();
                if (rt != null)
                {
                    rt.anchorMin = new Vector2(0f, 0.5f);
                    rt.anchorMax = new Vector2(0f, 0.5f);
                    rt.pivot = new Vector2(0f, 0.5f);
                    rt.sizeDelta = new Vector2(80, rt.sizeDelta.y);
                }
                // 清掉原 dropdown 的旧选项与监听，避免把分辨率选项带进来
                cloned.ClearOptions();
                cloned.onValueChanged.RemoveAllListeners();
                return cloned;
            }
            catch (Exception e)
            {
                Debug.LogWarning("[SimDLL_Rust] 克隆分辨率下拉异常: " + e.Message);
                return null;
            }
        }

        /// <summary>
        /// 翻译注册：Localization.Initialize 后注册 STRINGS + 加载 translations/&lt;locale&gt;.po。
        /// </summary>
        [HarmonyPatch(typeof(Localization), "Initialize")]
        public class LocalizationInitializePatch
        {
            public static void Postfix()
            {
                Localization.RegisterForTranslation(typeof(STRINGS));

                var locale = Localization.GetLocale();
                if (locale != null)
                {
                    string path = Path.Combine(
                        Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location) ?? ".",
                        "translations",
                        locale.Code + ".po");
                    if (File.Exists(path))
                    {
                        var strings = Localization.LoadStringsFile(path, false);
                        Localization.OverloadStrings(strings);

                        if (strings.TryGetValue("SimDLL_Rust.mod_description", out string descTranslation))
                        {
                            global::Strings.Add(new string[]
                            {
                                "Rust-based SimDLL",
                                descTranslation
                            });
                        }
                    }
                }

                LocString.CreateLocStringKeys(typeof(STRINGS), "SimDLL_Rust");
            }
        }
    }
}
