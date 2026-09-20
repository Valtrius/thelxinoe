-- The server owns segment policy and seeking; MPV owns the playback controls.
local mp = require 'mp'
local utils = require 'mp.utils'
local data = nil
local handled = {}
local current = nil

local function active_segment()
    if not data then return nil end
    local time = mp.get_property_number('time-pos')
    if not time then return nil end
    time = time + (data.timeline or 0)
    for _, segment in ipairs(data.items or {}) do
        local policy = data.preferences[segment.kind]
        if policy and policy ~= 'Ignore' and time >= segment.start and time < segment['end'] then
            return segment
        end
    end
end

local function skip()
    -- Resolve again so a keypress immediately after a seek does not use the
    -- previous timer tick's segment (or miss the newly entered segment).
    current = active_segment()
    if not current then return end
    handled[current.id] = true
    mp.commandv('script-message', 'thelxinoe-seek', data.file, data.generation, tostring(current['end']))
    mp.osd_message('', 0)
    current = nil
end

mp.register_script_message('thelxinoe-segments', function(text)
    local next_data = utils.parse_json(text)
    if not next_data then return end
    if not data or data.file ~= next_data.file or data.generation ~= next_data.generation then
        handled = {}
    end
    data = next_data
    current = nil
end)
mp.add_key_binding('Ctrl+ENTER', 'thelxinoe-skip-segment', skip)
mp.add_periodic_timer(0.2, function()
    current = active_segment()
    if current then
        local policy = data.preferences[current.kind]
        if policy == 'Auto' and not handled[current.id] and not mp.get_property_bool('pause') then
            skip()
        elseif policy == 'Ask' then
            mp.osd_message('Skip ' .. string.lower(current.kind) .. ' · Ctrl+Enter', 0.3)
        end
    end
end)
