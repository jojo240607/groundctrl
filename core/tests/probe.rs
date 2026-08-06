#[test]
fn print_mavmessage_type() {
    use mavlink::ardupilotmega::MavMessage;
    let msg = MavMessage::HEARTBEAT(mavlink::ardupilotmega::HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: mavlink::ardupilotmega::MavType::MAV_TYPE_QUADROTOR,
        autopilot: mavlink::ardupilotmega::MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
        base_mode: mavlink::ardupilotmega::MavModeFlag::empty(),
        system_status: mavlink::ardupilotmega::MavState::MAV_STATE_STANDBY,
        mavlink_version: 3,
    });
    // Reveal the fully-instantiated generic type name
    panic!("TYPE = {}", std::any::type_name_of_val(&msg));
}
