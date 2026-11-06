---
name: Burrow
description: A warm, flat workbench for running AI agents you can trust, after Intercom's visual system.
colors:
  bg: "#faf9f6"
  bg-dark: "#17100e"
  surface: "#ffffff"
  surface-dark: "#201815"
  chalk: "#f1eee9"
  border: "#dedbd6"
  border-dark: "#3a2f2a"
  ink: "#17100e"
  ink-dark: "#f4f3ec"
  muted-text: "#6b6560"
  muted-text-dark: "#a9a29b"
  brand: "#ff5600"
  focus: "#000ce1"
  danger: "#c41c1c"
  success: "#128a40"
  warning: "#6f5502"
typography:
  body:
    fontFamily: "Geist Variable, system-ui, sans-serif"
    fontSize: "14px"
    lineHeight: 1.5
  title:
    fontFamily: "Instrument Serif, Georgia, serif"
    fontWeight: 400
    letterSpacing: "-0.01em"
  label:
    fontFamily: "Geist Mono Variable, ui-monospace, monospace"
    fontSize: "12px"
    letterSpacing: "1.2px"
    textTransform: uppercase
rounded:
  base: "8px"
  control: "6px"
---

# Design System: Burrow

## Overview
Operate-mode desktop app. Warm off-white and near-black, flat with 1px linen borders, one loud colour kept for state. Light and dark follow the system.

## Colors
- **Ink:** warm black `#17100e` for text and primary buttons (cream `#f4f3ec` in dark).
- **Surfaces:** off-white page, white cards, chalk `#f1eee9` for hover and selection.
- **Brand orange `#ff5600`:** state only: the running lamp and approvals waiting on you. Black text on it, never white.
- **Focus:** electric blue ring (`#000ce1`, `#7a7eef` in dark), 2px with 2px offset.
- **Danger / success / warning:** meaning only, never decoration.

## Typography
Geist for the UI at 14px, weight 400 mostly. Instrument Serif for page titles only. Geist Mono, uppercase with 1.2px tracking, for small group labels. Fonts are bundled so all platforms match.

## Elevation and shape
Flat. Shadows only on overlays. Cards 8px, buttons, inputs and nav rows 6px, pills fully round. Lucide icons at 1.5 stroke.

## Source
Tokens read from intercom.com's published CSS. Its font files, logo, illustrations and icons are not used.
