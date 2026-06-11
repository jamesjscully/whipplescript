import type { JSX } from "@solidjs/web";

type IconProps = {
  size?: number;
  class?: string;
};

export type IconComponent = (props: IconProps) => JSX.Element;

function Icon(props: IconProps & { children?: JSX.Element }) {
  const size = () => props.size ?? 20;
  return (
    <svg
      class={props.class}
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.children}
    </svg>
  );
}

export const Activity: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M3 12h4l3-8 4 16 3-8h4" />
  </Icon>
);

export const AlertTriangle: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M12 3 2 21h20L12 3Z" />
    <path d="M12 9v5" />
    <path d="M12 18h.01" />
  </Icon>
);

export const Boxes: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M7 8 12 5l5 3-5 3-5-3Z" />
    <path d="M7 8v6l5 3 5-3V8" />
    <path d="M3 14l4-2 5 3v5l-5-3-4 2v-5Z" />
  </Icon>
);

export const CheckCircle2: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="12" cy="12" r="9" />
    <path d="m8 12 3 3 5-6" />
  </Icon>
);

export const ChevronLeft: IconComponent = (props) => (
  <Icon {...props}>
    <path d="m15 18-6-6 6-6" />
  </Icon>
);

export const ChevronRight: IconComponent = (props) => (
  <Icon {...props}>
    <path d="m9 18 6-6-6-6" />
  </Icon>
);

export const Clock3: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 7v5l3 2" />
  </Icon>
);

export const Database: IconComponent = (props) => (
  <Icon {...props}>
    <ellipse cx="12" cy="5" rx="8" ry="3" />
    <path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5" />
    <path d="M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7" />
  </Icon>
);

export const GitBranch: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="6" cy="5" r="3" />
    <circle cx="18" cy="19" r="3" />
    <circle cx="6" cy="19" r="3" />
    <path d="M6 8v8" />
    <path d="M9 19h6" />
  </Icon>
);

export const KeyRound: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="7.5" cy="14.5" r="4.5" />
    <path d="M11 11 21 1" />
    <path d="m16 6 2 2" />
    <path d="m19 3 2 2" />
  </Icon>
);

export const Lock: IconComponent = (props) => (
  <Icon {...props}>
    <rect x="4" y="10" width="16" height="11" rx="2" />
    <path d="M8 10V7a4 4 0 0 1 8 0v3" />
  </Icon>
);

export const LogOut: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M10 17 15 12 10 7" />
    <path d="M15 12H3" />
    <path d="M21 3v18" />
  </Icon>
);

export const PauseCircle: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="12" cy="12" r="9" />
    <path d="M10 8v8" />
    <path d="M14 8v8" />
  </Icon>
);

export const Play: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M8 5v14l11-7-11-7Z" />
  </Icon>
);

export const RefreshCw: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M20 12a8 8 0 0 1-14 5" />
    <path d="M4 12a8 8 0 0 1 14-5" />
    <path d="M18 3v4h-4" />
    <path d="M6 21v-4h4" />
  </Icon>
);

export const Route: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="6" cy="6" r="3" />
    <circle cx="18" cy="18" r="3" />
    <path d="M9 6h4a5 5 0 0 1 0 10H9" />
  </Icon>
);

export const Search: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-4-4" />
  </Icon>
);

export const Server: IconComponent = (props) => (
  <Icon {...props}>
    <rect x="4" y="4" width="16" height="6" rx="2" />
    <rect x="4" y="14" width="16" height="6" rx="2" />
    <path d="M8 7h.01" />
    <path d="M8 17h.01" />
  </Icon>
);

export const Settings: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.4 1a8 8 0 0 0-2-1.1L14 3h-4l-.5 2.8a8 8 0 0 0-2 1.1l-2.4-1-2 3.4 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.4 2.4-1a8 8 0 0 0 2 1.1L10 21h4l.5-2.8a8 8 0 0 0 2-1.1l2.4 1 2-3.4-2-1.5c.1-.4.1-.8.1-1.2Z" />
  </Icon>
);

export const ShieldCheck: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M12 3 20 6v6c0 5-3.4 8-8 9-4.6-1-8-4-8-9V6l8-3Z" />
    <path d="m8.5 12 2 2 5-5" />
  </Icon>
);

export const TimerReset: IconComponent = (props) => (
  <Icon {...props}>
    <path d="M10 2h4" />
    <path d="M12 14v-4" />
    <path d="M4 13a8 8 0 1 0 2.3-5.7" />
    <path d="M4 5v4h4" />
  </Icon>
);

export const Workflow: IconComponent = (props) => (
  <Icon {...props}>
    <rect x="3" y="4" width="6" height="6" rx="1.5" />
    <rect x="15" y="4" width="6" height="6" rx="1.5" />
    <rect x="9" y="15" width="6" height="6" rx="1.5" />
    <path d="M9 7h6" />
    <path d="M12 10v5" />
  </Icon>
);

export const XCircle: IconComponent = (props) => (
  <Icon {...props}>
    <circle cx="12" cy="12" r="9" />
    <path d="m9 9 6 6" />
    <path d="m15 9-6 6" />
  </Icon>
);
