#!/usr/bin/env lua5.4
-- Audit cards that declare ability text but have no engine implementation
-- (no on_* handler, no static block, no activated block, and the ability
-- isn't a recognized bare keyword).

local KNOWN_KEYWORDS = {
  ["flying."] = true, ["flying"] = true,
  ["haste."] = true, ["haste"] = true,
  ["vigilance."] = true, ["vigilance"] = true,
  ["defender."] = true, ["defender"] = true,
  ["unblockable."] = true, ["unblockable"] = true,
  ["reach."] = true, ["reach"] = true,
  ["cannot-block."] = true, ["cannot-block"] = true,
}

local function is_keyword_only(text)
  return KNOWN_KEYWORDS[text:lower():match("^%s*(.-)%s*$")] == true
end

local function list_lua_files(dir)
  local files = {}
  local p = io.popen("ls " .. dir .. "/*.lua 2>/dev/null")
  if not p then return files end
  for line in p:lines() do table.insert(files, line) end
  p:close()
  table.sort(files)
  return files
end

local cards_dir = arg[1] or "cards"
local rows = {}
for _, path in ipairs(list_lua_files(cards_dir)) do
  local ok, card = pcall(dofile, path)
  if ok and type(card) == "table" then
    local has_impl =
      card.on_play or card.on_die or card.on_attack or card.on_block
      or card.on_blocked_by or card.on_enter_board or card.on_attached_as_cost
      or card.static or (card.activated and #card.activated > 0)
    if not has_impl and type(card.abilities) == "table" and #card.abilities > 0 then
      -- Filter out cards whose abilities are entirely bare keywords.
      local nontrivial = {}
      for _, a in ipairs(card.abilities) do
        if not is_keyword_only(a) then table.insert(nontrivial, a) end
      end
      if #nontrivial > 0 then
        table.insert(rows, {
          id = card.id or path,
          abilities = nontrivial,
        })
      end
    end
  end
end

print(string.format("%d card(s) with declared abilities but no implementation:", #rows))
print()
for _, r in ipairs(rows) do
  print(string.format("  %s", r.id))
  for _, a in ipairs(r.abilities) do
    print(string.format("    - %s", a))
  end
end
