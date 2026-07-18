import {
  Crop28Regular,
  Keyboard24Regular,
  Pin24Regular,
  ScanQrCode24Regular,
  DataPie24Regular
} from "@fluentui/react-icons";
import type { ComponentType, SVGProps } from "react";
import type { GlobalHotkeyId, HotkeyState } from "../../app/hotkeyModel";
import { ClipboardWithLinesIcon } from "../icons/ClipboardWithLinesIcon";
import { ShortcutBadge, StatusBadge } from "../primitives/Badge";
import { Button } from "../primitives/Button";
import { Toggle } from "../primitives/SelectionControl";
import { List, ListRow, ListRowDescription, ListRowTitle } from "./ListRow";
import styles from "./HotkeyList.module.css";

type HotkeyDensity = "overview" | "full";
type HotkeyIcon = ComponentType<SVGProps<SVGSVGElement>>;

export type HotkeyListItem = {
  id: GlobalHotkeyId;
  title: string;
  description: string;
  binding: string | null;
  state: HotkeyState;
  detail?: string | null;
  enabled?: boolean | null;
};

const hotkeyIcons: Record<GlobalHotkeyId, HotkeyIcon> = {
  capture: Crop28Regular,
  pinImage: Pin24Regular,
  clipboardQr: ScanQrCode24Regular,
  toolWheel: DataPie24Regular,
  clipboardPanel: ClipboardWithLinesIcon
};

type HotkeyRowProps = {
  hotkey: HotkeyListItem;
  density?: HotkeyDensity;
  onEnabledChange?: (id: GlobalHotkeyId, checked: boolean) => void;
  onEdit?: (id: GlobalHotkeyId) => void;
  toggleDisabled?: boolean | ((hotkey: HotkeyListItem) => boolean);
  editDisabled?: boolean | ((hotkey: HotkeyListItem) => boolean);
};

function resolveDisabled(disabled: HotkeyRowProps["toggleDisabled"], hotkey: HotkeyListItem) {
  return typeof disabled === "function" ? disabled(hotkey) : Boolean(disabled);
}

export function HotkeyRow({
  hotkey,
  density = "full",
  onEnabledChange,
  onEdit,
  toggleDisabled = false,
  editDisabled = false
}: HotkeyRowProps) {
  const Icon = hotkeyIcons[hotkey.id] ?? Keyboard24Regular;
  const isToggleDisabled = resolveDisabled(toggleDisabled, hotkey) || !onEnabledChange;
  const isEditDisabled = resolveDisabled(editDisabled, hotkey) || !onEdit;

  return (
    <ListRow className={[styles.hotkeyRow, styles[`hotkeyRow${density}`]].join(" ")}>
      <span className={styles.badgeCell}>
        <ShortcutBadge value={hotkey.binding} />
      </span>
      <span className={styles.hotkeyIcon} aria-hidden="true">
        <Icon className={styles.featureIcon} aria-hidden={true} />
      </span>
      <div className={styles.hotkeyCopy}>
        <ListRowTitle>{hotkey.title}</ListRowTitle>
        <ListRowDescription>{hotkey.description}</ListRowDescription>
      </div>
      <span className={styles.statusCell}>
        <StatusBadge state={hotkey.state} detail={hotkey.detail ?? null} />
      </span>
      <span className={styles.toggleCell}>
        <Toggle
          checked={hotkey.enabled ?? null}
          label={`${hotkey.title}快捷键`}
          disabled={isToggleDisabled}
          onChange={(checked) => onEnabledChange?.(hotkey.id, checked)}
        />
      </span>
      <Button
        className={styles.hotkeyEdit}
        size="inline"
        disabled={isEditDisabled}
        onClick={() => onEdit?.(hotkey.id)}
      >
        编辑
      </Button>
    </ListRow>
  );
}

type HotkeyListProps = {
  hotkeys: HotkeyListItem[];
  density?: HotkeyDensity;
  limit?: number;
  className?: string;
  onEnabledChange?: (id: GlobalHotkeyId, checked: boolean) => void;
  onEdit?: (id: GlobalHotkeyId) => void;
  toggleDisabled?: boolean | ((hotkey: HotkeyListItem) => boolean);
  editDisabled?: boolean | ((hotkey: HotkeyListItem) => boolean);
};

export function HotkeyList({
  hotkeys,
  density = "full",
  limit,
  className = "",
  onEnabledChange,
  onEdit,
  toggleDisabled,
  editDisabled
}: HotkeyListProps) {
  const visibleHotkeys = typeof limit === "number" ? hotkeys.slice(0, limit) : hotkeys;

  return (
    <List className={[styles.hotkeyList, styles[`hotkeyList${density}`], className].filter(Boolean).join(" ")}>
      {visibleHotkeys.map((hotkey) => (
        <HotkeyRow
          hotkey={hotkey}
          density={density}
          key={hotkey.id}
          onEnabledChange={onEnabledChange}
          onEdit={onEdit}
          toggleDisabled={toggleDisabled}
          editDisabled={editDisabled}
        />
      ))}
    </List>
  );
}
