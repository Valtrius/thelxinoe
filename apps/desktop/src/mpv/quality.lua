local mp = require 'mp'
local utils = require 'mp.utils'
local current = nil
local script = mp.get_script_name()

local function open_menu()
    if not current or not current.qualities or #current.qualities == 0 then return end
    local items = {}
    for _, quality in ipairs(current.qualities) do
        items[#items + 1] = {
            title = quality.label,
            active = quality.value == current.selected,
            value = {'script-message', 'thelxinoe-quality', current.session, quality.value},
        }
    end
    mp.commandv('script-message-to', 'uosc', 'open-menu', utils.format_json({
        id = 'thelxinoe-quality', title = 'Quality', items = items,
    }))
end

local function configure()
    if current and current.qualities and #current.qualities > 0 then
        mp.commandv('script-message-to', 'uosc', 'overwrite-binding', 'stream-quality',
            'script-binding ' .. script .. '/open')
    else
        mp.commandv('script-message-to', 'uosc', 'overwrite-binding', 'stream-quality')
    end
end

mp.add_key_binding(nil, 'open', open_menu)
mp.register_script_message('thelxinoe-qualities', function(payload)
    current = utils.parse_json(payload)
    configure()
end)
mp.register_script_message('uosc-version', configure)
mp.commandv('script-message-to', 'uosc', 'get-version', script)
