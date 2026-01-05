-- Example ovim configuration file
-- Place this file at one of the following locations:
--   $OVIM_CONFIG/init.lua
--   $XDG_CONFIG_HOME/ovim/init.lua
--   ~/.config/ovim/init.lua
--   ~/.ovim/init.lua

-- Set options using vim.opt (Neovim-style)
vim.opt.number = true              -- Show line numbers
vim.opt.relativenumber = true      -- Show relative line numbers
vim.opt.expandtab = true           -- Use spaces instead of tabs
vim.opt.tabstop = 4                -- Tab width
vim.opt.shiftwidth = 4             -- Indent width
vim.opt.scroll = 10                -- Half-page scroll amount

-- Or use vim.cmd to execute ex commands
vim.cmd('set number')
vim.cmd('set relativenumber')
vim.cmd('colorscheme tokyonight')

-- You can also use vim.api functions
vim.api.nvim_command('set tabstop=4')

-- Print a message
print("Configuration loaded!")

-- Example: Custom function
local function setup_editor()
    vim.opt.number = true
    vim.opt.relativenumber = false
    print("Editor configured")
end

-- Call your function
setup_editor()
