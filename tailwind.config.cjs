/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        app: "var(--app)",
        sidebar: "var(--sidebar)",
        subtle: "var(--subtle)",
        selected: "var(--selected)",
        surface: "var(--surface)",
        border: "var(--border)",
        "border-strong": "var(--border-strong)",
        fg: {
          1: "var(--fg-1)",
          2: "var(--fg-2)",
          3: "var(--fg-3)",
        },
        action: {
          DEFAULT: "var(--action)",
          hover: "var(--action-hover)",
          text: "var(--action-text)",
          soft: "var(--action-soft)",
        },
        success: {
          DEFAULT: "var(--success)",
          text: "var(--success-text)",
          soft: "var(--success-soft)",
        },
        warning: {
          DEFAULT: "var(--warning)",
          text: "var(--warning-text)",
          soft: "var(--warning-soft)",
        },
        danger: {
          DEFAULT: "var(--danger)",
          text: "var(--danger-text)",
          soft: "var(--danger-soft)",
        },
        ring: "var(--ring)",
      },
      borderRadius: {
        control: "6px",
        panel: "10px",
        dialog: "14px",
      },
      fontSize: {
        badge: ["11px", { lineHeight: "16px", fontWeight: "500" }],
        caption: ["12px", { lineHeight: "18px" }],
        body: ["13px", { lineHeight: "20px" }],
        strong: ["14px", { lineHeight: "20px", fontWeight: "500" }],
        section: ["15px", { lineHeight: "22px", fontWeight: "600" }],
        title: ["16px", { lineHeight: "24px", fontWeight: "600" }],
        page: ["18px", { lineHeight: "26px", fontWeight: "600" }],
      },
    },
  },
  plugins: [],
};