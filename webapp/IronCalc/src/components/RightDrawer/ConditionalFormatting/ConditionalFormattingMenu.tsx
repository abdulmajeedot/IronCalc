import type { CfRuleInput, Cfvo, Color, IconThreshold } from "@ironcalc/wasm";
import { iconSetThresholds } from "@ironcalc/wasm";
import {
  CalendarDays,
  ChartBarDecreasing,
  Copy,
  Equal,
  Eraser,
  Grid2x2,
  Highlighter,
  ListOrdered,
  ListTree,
  Plus,
  Search,
  Shapes,
} from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { iconSpecFor } from "../../IconPicker/IconPicker";
import { Menu } from "../../Menu/Menu";
import { MenuDivider } from "../../Menu/MenuDivider";
import { MenuItem, MenuItemWithSubmenu } from "../../Menu/MenuItem";
import type { RuleData } from "./EditRule";
import "./conditional-formatting.css";

export type CfMenuAction =
  | { type: "edit"; preset: Partial<RuleData> }
  | { type: "apply"; rule: CfRuleInput }
  | { type: "new" }
  | { type: "manage" }
  | { type: "clear"; scope: "selection" | "sheet" };

// Excel's default look for these rules: light red fill with dark red text
const LIGHT_RED = {
  bold: false,
  italic: false,
  underline: false,
  strike: false,
  fontColor: "#9C0006",
  fillColor: "#FFC7CE",
};

const BAR_COLORS = [
  "#638EC6",
  "#63C384",
  "#FF555A",
  "#FFB628",
  "#008AEF",
  "#D6007B",
];

const RED = "#F8696B";
const YELLOW = "#FFEB84";
const PALE_YELLOW = "#FFEF9C";
const GREEN = "#63BE7B";
const WHITE = "#FCFCFF";
const BLUE = "#5A8AC6";

// Lowest value's color first, as in the rule; shown with the highest on top
const COLOR_SCALES: string[][] = [
  [RED, YELLOW, GREEN],
  [GREEN, YELLOW, RED],
  [RED, WHITE, GREEN],
  [GREEN, WHITE, RED],
  [RED, WHITE, BLUE],
  [BLUE, WHITE, RED],
  [RED, WHITE],
  [WHITE, RED],
  [WHITE, GREEN],
  [GREEN, WHITE],
  [PALE_YELLOW, GREEN],
  [GREEN, PALE_YELLOW],
];

const ICON_GROUPS: { label: string; sets: string[] }[] = [
  {
    label: "directional",
    sets: [
      "3Arrows",
      "3ArrowsGray",
      "3Triangles",
      "4ArrowsGray",
      "4Arrows",
      "5ArrowsGray",
      "5Arrows",
    ],
  },
  {
    label: "shapes",
    sets: ["3TrafficLights1", "3Signs", "4TrafficLights", "4RedToBlack"],
  },
  { label: "indicators", sets: ["3Symbols", "3Flags"] },
];

function colorScaleRule(colors: string[]): CfRuleInput {
  const stops =
    colors.length === 2
      ? (["Min", "Max"] as const)
      : (["Min", { Percentile: 50 }, "Max"] as const);
  return {
    type: "ColorScale",
    thresholds: colors.map((color, index) => ({
      cfvo: stops[index],
      color: color as Color,
    })),
  };
}

function dataBarRule(color: string, gradient: boolean): CfRuleInput {
  return {
    type: "DataBar",
    min: null,
    max: null,
    positive_color: color as Color,
    negative_color: "#FF0000" as Color,
    is_gradient: gradient,
    show_value: true,
  };
}

const colorOf = (color: Color): string =>
  typeof color === "string" ? color : "#808080";

const ConditionalFormattingMenu = (props: {
  anchor: { x: number; y: number };
  onClose: () => void;
  onAction: (action: CfMenuAction) => void;
}) => {
  const { anchor, onClose, onAction } = props;
  const { t } = useTranslation();
  const label = (key: string) => t(`conditional_formatting.menu_${key}`);

  const iconSets = useMemo(() => {
    const sets = new Map<string, IconThreshold[]>();
    for (const group of ICON_GROUPS) {
      for (const name of group.sets) {
        sets.set(name, iconSetThresholds(name) ?? []);
      }
    }
    return sets;
  }, []);

  const act = (action: CfMenuAction): void => {
    onClose();
    onAction(action);
  };

  const editItem = (
    key: string,
    icon: ReactNode,
    preset: Partial<RuleData>,
  ) => (
    <MenuItem
      key={key}
      icon={icon}
      onClick={() =>
        act({
          type: "edit",
          preset: { formatStyle: LIGHT_RED, ruleValue: "", ...preset },
        })
      }
    >
      {label(key)}
    </MenuItem>
  );

  const cellValue = (ruleOperator: string) => ({
    ruleType: "cell_value",
    ruleOperator,
  });
  const topBottom = (ruleOperator: string, ruleValue = "10") => ({
    ruleType: "top_bottom",
    ruleOperator,
    ruleValue,
  });

  const bars = (gradient: boolean) => (
    <div className="ic-cf-gallery">
      {BAR_COLORS.map((color) => (
        <button
          key={color}
          type="button"
          className="ic-cf-gallery-item ic-cf-bar-swatch"
          aria-label={`${gradient ? "gradient" : "solid"} ${color}`}
          onClick={() =>
            act({ type: "apply", rule: dataBarRule(color, gradient) })
          }
        >
          {[90, 60, 75, 35].map((width) => (
            <span
              key={width}
              style={{
                width: `${width}%`,
                background: gradient
                  ? `linear-gradient(to right, ${color}, white)`
                  : color,
              }}
            />
          ))}
        </button>
      ))}
    </div>
  );

  return (
    <Menu open onClose={onClose} anchorPosition={anchor}>
      <MenuItemWithSubmenu
        icon={<Highlighter />}
        submenu={
          <>
            {editItem("greater_than", <ListTree />, cellValue("greater_than"))}
            {editItem("less_than", <ListTree />, cellValue("less_than"))}
            {editItem("between", <ListTree />, cellValue("between"))}
            {editItem("equal_to", <Equal />, cellValue("equals"))}
            {editItem("text_contains", <Search />, {
              ruleType: "text",
              ruleOperator: "contains",
            })}
            {editItem("date_occurring", <CalendarDays />, {
              ruleType: "date",
              ruleOperator: "yesterday",
            })}
            {editItem("duplicate_values", <Copy />, {
              ruleType: "duplicate_values",
              ruleOperator: "",
            })}
          </>
        }
      >
        {label("highlight_rules")}
      </MenuItemWithSubmenu>
      <MenuItemWithSubmenu
        icon={<ListOrdered />}
        submenu={
          <>
            {editItem("top_items", <ListOrdered />, topBottom("top_items"))}
            {editItem("top_percent", <ListOrdered />, topBottom("top_percent"))}
            {editItem(
              "bottom_items",
              <ListOrdered />,
              topBottom("bottom_items"),
            )}
            {editItem(
              "bottom_percent",
              <ListOrdered />,
              topBottom("bottom_percent"),
            )}
            {editItem(
              "above_average",
              <ListOrdered />,
              topBottom("above_average", ""),
            )}
            {editItem(
              "below_average",
              <ListOrdered />,
              topBottom("below_average", ""),
            )}
          </>
        }
      >
        {label("top_bottom_rules")}
      </MenuItemWithSubmenu>

      <MenuDivider />

      <MenuItemWithSubmenu
        icon={<ChartBarDecreasing />}
        submenu={
          <div className="ic-cf-gallery-panel">
            <div className="ic-cf-gallery-title">{label("gradient_fill")}</div>
            {bars(true)}
            <div className="ic-cf-gallery-title">{label("solid_fill")}</div>
            {bars(false)}
          </div>
        }
      >
        {label("data_bars")}
      </MenuItemWithSubmenu>
      <MenuItemWithSubmenu
        icon={<Grid2x2 />}
        submenu={
          <div className="ic-cf-gallery-panel">
            <div className="ic-cf-gallery ic-cf-gallery--four">
              {COLOR_SCALES.map((colors) => (
                <button
                  key={colors.join("-")}
                  type="button"
                  className="ic-cf-gallery-item ic-cf-scale-swatch"
                  aria-label={colors.join(" ")}
                  onClick={() =>
                    act({ type: "apply", rule: colorScaleRule(colors) })
                  }
                >
                  {[...colors].reverse().map((color) => (
                    <span key={color} style={{ background: color }} />
                  ))}
                </button>
              ))}
            </div>
          </div>
        }
      >
        {label("color_scales")}
      </MenuItemWithSubmenu>
      <MenuItemWithSubmenu
        icon={<Shapes />}
        submenu={
          <div className="ic-cf-gallery-panel">
            {ICON_GROUPS.map((group) => (
              <div key={group.label}>
                <div className="ic-cf-gallery-title">{label(group.label)}</div>
                <div className="ic-cf-gallery ic-cf-gallery--icons">
                  {group.sets.map((name) => {
                    const thresholds = iconSets.get(name) ?? [];
                    return (
                      <button
                        key={name}
                        type="button"
                        className="ic-cf-gallery-item ic-cf-icon-swatch"
                        aria-label={name}
                        onClick={() =>
                          act({
                            type: "apply",
                            rule: {
                              type: "IconSet",
                              thresholds,
                              show_value: true,
                            },
                          })
                        }
                      >
                        {[...thresholds].reverse().map((threshold, index) => {
                          const { Icon, filled } = iconSpecFor(threshold.icon);
                          const color = colorOf(threshold.color);
                          return (
                            <Icon
                              // biome-ignore lint/suspicious/noArrayIndexKey: a set can repeat an icon
                              key={index}
                              size={13}
                              color={color}
                              fill={filled ? color : "none"}
                            />
                          );
                        })}
                      </button>
                    );
                  })}
                </div>
              </div>
            ))}
            <div className="ic-cf-gallery-title">{label("ratings")}</div>
            <div className="ic-cf-gallery ic-cf-gallery--icons">
              {([3, 4, 5] as const).map((count) => {
                const star = iconSpecFor("Star");
                return (
                  <button
                    key={count}
                    type="button"
                    className="ic-cf-gallery-item ic-cf-icon-swatch"
                    aria-label={`${count} stars`}
                    onClick={() =>
                      act({
                        type: "apply",
                        rule: {
                          type: "IconRating",
                          icon: "Star",
                          color: "#F8CD3D" as Color,
                          thresholds: Array.from(
                            { length: count },
                            (_, index): [Cfvo, boolean] => [
                              {
                                Percent: Math.round((index * 100) / count),
                              },
                              true,
                            ],
                          ),
                          show_value: true,
                        },
                      })
                    }
                  >
                    {Array.from({ length: count }, (_, index) => (
                      <star.Icon
                        // biome-ignore lint/suspicious/noArrayIndexKey: identical icons
                        key={index}
                        size={13}
                        color="#F8CD3D"
                        fill="#F8CD3D"
                      />
                    ))}
                  </button>
                );
              })}
            </div>
          </div>
        }
      >
        {label("icon_sets")}
      </MenuItemWithSubmenu>

      <MenuDivider />

      <MenuItem icon={<Plus />} onClick={() => act({ type: "new" })}>
        {label("new_rule")}
      </MenuItem>
      <MenuItemWithSubmenu
        icon={<Eraser />}
        submenu={
          <>
            <MenuItem
              onClick={() => act({ type: "clear", scope: "selection" })}
            >
              {label("clear_selection")}
            </MenuItem>
            <MenuItem onClick={() => act({ type: "clear", scope: "sheet" })}>
              {label("clear_sheet")}
            </MenuItem>
          </>
        }
      >
        {label("clear_rules")}
      </MenuItemWithSubmenu>
      <MenuItem icon={<ListTree />} onClick={() => act({ type: "manage" })}>
        {label("manage_rules")}
      </MenuItem>
    </Menu>
  );
};

export default ConditionalFormattingMenu;
