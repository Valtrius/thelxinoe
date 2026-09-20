-- Integration for the app-managed uosc installation. This is loaded only in a
-- new player's configuration snapshot, alongside the unmodified catalog plugin.
local mp = require 'mp'
local managed_directory = __MANAGED_CONFIG_JSON__

mp.register_script_message('open-managed-config', function()
    mp.command_native({
        name = 'subprocess',
        args = {'explorer.exe', managed_directory},
        playback_only = false,
    })
end)

-- Scripts initialize asynchronously. Repeat briefly so uosc has registered its
-- documented overwrite-binding message before we send the integration commands.
local attempts = 0
local timer
timer = mp.add_periodic_timer(0.1, function()
    mp.commandv('script-message-to', 'uosc', 'overwrite-binding', 'update',
        'show-text "Manage uosc updates in Thelxinoe > Settings > MPV." 5000')
    mp.commandv('script-message-to', 'uosc', 'overwrite-binding', 'open-config-directory',
        'script-message-to ' .. mp.get_script_name() .. ' open-managed-config')
    attempts = attempts + 1
    if attempts >= 10 then timer:kill() end
end)
