"use client";

import React, { useState } from "react";

import { CommandPalette } from "./CommandPalette";

/**
 * Client wrapper that owns the palette's open state so the Pages Router
 * `_app.tsx` can drop it in without managing state itself (WEB-52 / #185).
 */
export function CommandPaletteLauncher() {
  const [open, setOpen] = useState(false);

  return <CommandPalette open={open} onOpenChange={setOpen} />;
}
