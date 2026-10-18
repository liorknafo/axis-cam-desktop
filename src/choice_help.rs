//! Help for the selected value, using the same resolved defaults as the controls.
use crate::{
    Target,
    help::{Diagram, Help},
    model::{Fields, facing_setting},
};

/// What More… adds to each stock-detail choice: how the surface is meshed.
const STOCK_DETAIL_MORE: &str = "Adaptive octree meshing keeps uniform regions coarse and refines the surface. Dual contouring preserves planes and cutter-radius curves. Each detail step doubles sampling resolution per axis. This is display resolution, not machining tolerance; features thinner than a cell can be missed. Updates run in the background. Updating means the last completed surface is still shown. Removed volume is a separate coarse estimate.";
pub fn selected(key: &str, label: &str, value: &str, target: Target, p: &Fields) -> Help {
    let mut diagram = None;
    let (body, more) = match (key, value) {
        ("spindle_rotation", "Clockwise (CW)") => {
            diagram = Some(Diagram::Spindle(true));
            (
                "The spindle turns clockwise when viewed from the spindle toward the part along setup −Z. Use a matching cutter. The spindle keeps this rotation when feed reverses; zigzag Facing still mixes climb and conventional cutting.",
                "",
            )
        }
        ("spindle_rotation", "Counterclockwise (CCW)") => {
            diagram = Some(Diagram::Spindle(false));
            (
                "The spindle turns counterclockwise when viewed from the spindle toward the part along setup −Z. Use a matching cutter. Compared with CW, one-way Facing and closed contours reverse their feed direction to keep the selected milling method.",
                "",
            )
        }
        ("facing_milling" | "pocket_milling", "Climb") => {
            diagram = Some(Diagram::MillingChoice(
                true,
                facing_setting(p, "spindle_rotation") == "Clockwise (CW)",
            ));
            (
                "The chip starts thicker and exits thinner. Feed direction is chosen relative to the stock and the saved spindle rotation.",
                "Entry and full-width cuts may contain both cutting modes.",
            )
        }
        ("facing_milling" | "pocket_milling", "Conventional") => {
            diagram = Some(Diagram::MillingChoice(
                false,
                facing_setting(p, "spindle_rotation") == "Clockwise (CW)",
            ));
            (
                "The chip starts thin and grows thicker. This reverses the corresponding Climb path while keeping the saved spindle rotation.",
                "Entry and full-width cuts may contain both modes.",
            )
        }
        ("facing_pattern", "Both ways (zigzag)") => {
            diagram = Some(Diagram::FacingPattern(true));
            (
                "Cut one row, then reverse feed along the next row. The spindle does not reverse. This combines climb and conventional cutting. Next depth level and Between facing passes control the layer transition and whether the tool stays down.",
                "",
            )
        }
        ("facing_pattern", "One way") => {
            diagram = Some(Diagram::FacingPattern(false));
            (
                "Every cutting row travels in the same direction, using the selected Climb or Conventional method. Return moves retract to clearance. Every new depth starts at the first row; keep-down and continue-from-end settings do not apply.",
                "",
            )
        }
        ("facing_depth_order", "Continue from previous end") => (
            "At the next depth, start at the last pass's X/Y and reverse the complete row sequence. With Keep tool down, feed vertically down there before cutting back. Applies to Both ways; One way always returns to the first start.",
            "",
        ),
        ("facing_depth_order", "Return to first start") => (
            "After finishing a depth, retract and return to the original X/Y start. Repeat the same row order at the next depth. This adds return travel compared with continuing from the previous endpoint.",
            "",
        ),
        ("facing_link", "Keep tool down") => {
            diagram = Some(Diagram::FacingLink(true));
            (
                "Feed sideways to the neighboring row at cutting depth. With Continue from previous end, descend at the last X/Y for the next layer. Applies to Both ways inside the rectangular facing extent; fixtures are not avoided, and a move within the clearance gap of a modelled vise jaw or clamp is refused at export.",
                "",
            )
        }
        ("facing_link", "Retract each pass") => {
            diagram = Some(Diagram::FacingLink(false));
            (
                "After each cutting row, lift to clearance, reposition in XY, then feed down for the next row. This adds non-cutting travel. It does not route around fixtures; a move within the clearance gap of a modelled vise jaw or clamp is warned of in the preview and refused at export.",
                "",
            )
        }
        ("pocket_entry", "Profile ramp") => {
            diagram = Some(Diagram::Entry(true));
            (
                "Descend along the closed contour at the selected ramp angle, then complete a loop at depth. Entry uses 25% of cutting feed. Transfers between contours retract to clearance so they do not cross islands at cutting depth.",
                "",
            )
        }
        ("pocket_entry", "Plunge") => {
            diagram = Some(Diagram::Entry(false));
            (
                "Feed straight down at the contour start using 25% of cutting feed, then follow the contour at depth. Use a plunge-capable cutter. This is a direct vertical entry, with no helical or sloping approach.",
                "",
            )
        }
        ("cycle", "Drilling") => {
            diagram = Some(Diagram::DrillCycle(0));
            (
                "Feed directly to the target depth once, then retract. There are no intermediate pecks or bottom dwell in this cycle.",
                "",
            )
        }
        ("cycle", "Chip breaking") => {
            diagram = Some(Diagram::DrillCycle(1));
            (
                "Drill in increments set by Peck depth. Make a short retract between increments to break the chip, then continue deeper. The tool does not retract fully out of the hole after every peck.",
                "",
            )
        }
        ("cycle", "Deep drilling") => {
            diagram = Some(Diagram::DrillCycle(2));
            (
                "Drill by Peck depth, retract clear between pecks, then re-enter before continuing deeper. This gives chips a chance to clear but adds travel compared with Chip breaking.",
                "",
            )
        }
        ("cycle", "Counterboring") => {
            diagram = Some(Diagram::DrillCycle(3));
            (
                "Feed to the target, pause for the selected Dwell time at the bottom, then retract. The preview removes the drill's flutes and its 118° point, or a chamfer mill's cone.",
                "",
            )
        }
        ("on_pins", "Yes") => (
            "The part drops over the machine's fixture pins by holes in it, so its place and turn are known: the pins give this setup's X and Y work offset (Set X Y from the pins on the Control page), with no probing. Z is still touched off on the stock. The pins are kept clear of as workholding; the vise or clamps still hold the part down.",
            "Cycle Start waits until X and Y are referenced from pin 1 since the last E-stop and the work offset is the one the pins give. The check warns when no earlier setup drills the holes for the pins.",
        ),
        ("on_pins", "No") => (
            "The part does not sit on the fixture pins: its X and Y work zero is touched off or probed on the stock.",
            "",
        ),
        ("tabs", "No") => (
            "The contour goes all the way round at every level. Where it cuts through the stock's bottom the part comes free of the stock around it, and can be thrown by the cutter: leave a skin with Bottom offset, or turn tabs on.",
            "",
        ),
        ("tabs", "Yes") => (
            "Leave tabs: short bridges of material across the cut at its bottom that keep the part joined to the stock around it after the contour goes through, so the cutter cannot throw it. They only hold where stock is left beyond the cut: leave more beside the part than the stock to leave, the tool's diameter and a 2 mm wall.",
            "Generate refuses tabs that would join the part to no stock. Below the tabs' top the tool rises over each tab and plunges back down after it; the levels above go round in full. The tabs stay on the part: cut or file them off by hand afterwards.",
        ),
        ("openings", "Leave") => (
            "Clear the selected floors only. Holes through a floor are cleared over at the floor's height and left for another operation.",
            "",
        ),
        ("openings", "Cut through") => (
            "Clear the selected floors, then cut each hole through them that nothing of the part covers or underlies down to where its walls end, plus Breakthrough. Openings narrower than the cutter are left for a smaller one.",
            "Each opening's walls keep the cutter's radius in their corners, as a pocket's do. The extra depth goes into the stock under the part; modelled vise jaws and clamps are checked, parallels and the table are not.",
        ),
        ("through", "Yes") => (
            "Extend the drill target below the bottom of each hole that goes through the part by Breakthrough. A selected hole that is blind is refused, since the part goes on under its bottom: set Through to No, or use the diameter filter to drill only the holes that go through.",
            "That extra travel is included in the preview, and modelled vise jaws and clamps are checked as boxes with a clearance gap; the material, parallels or the table underneath are not collision checked.",
        ),
        ("through", "No") => (
            "Stop at the selected bottom plus Bottom offset, for blind holes and holes that go through alike. The Breakthrough distance is not added. This does not guarantee a full-diameter exit for a real conical drill tip.",
            "",
        ),
        ("hole_order", "Optimize travel") => (
            "Visit the next nearby detected hole using a greedy travel order. This reduces local linking distance; it is not a global minimum-time route calculation.",
            "",
        ),
        ("hole_order", "By X then Y") => (
            "Sort detected hole centers by setup X, then Y. This gives a predictable geometric sequence independent of the order in which rim faces were clicked.",
            "",
        ),
        ("boundary", "Selected geometry") => (
            "Use the regions selected on the current imported part. Facing uses their XY bounding rectangle. Pocket and Inside Contour use selected recessed floor boundaries and preserve islands. Select geometry appropriate to this operation.",
            "",
        ),
        ("boundary", "Stock silhouette") => (
            "Use the starting stock's projected XY extent as the facing boundary. This can cover extra material outside the model. Cutter containment determines whether the center stays inside or extends beyond that extent.",
            "",
        ),
        ("boundary", "Model silhouette") => (
            "Use the current placed model's projected XY extent. Facing uses a bounding rectangle; Outside Contour follows the projected outline. Parallel and Waterline use the whole imported model with their respective vertical-access rules.",
            "",
        ),
        ("containment", "Tool inside boundary") => {
            diagram = Some(Diagram::Containment(-1));
            (
                "Keep the cutter's outer edge inside the boundary by moving its center inward by the tool radius. Narrow regions that cannot fit the cutter remain uncut.",
                "",
            )
        }
        ("containment", "Tool center on boundary") => {
            diagram = Some(Diagram::Containment(0));
            (
                "Allow the cutter center to reach the boundary. Half the cutter can extend outside it, so the machined area reaches one radius farther than the center path.",
                "",
            )
        }
        ("containment", "Tool outside boundary") => {
            diagram = Some(Diagram::Containment(1));
            (
                "Allow the cutter center one radius beyond the boundary. This lets a facing cutter fully pass an outside edge. The tool's outer edge extends farther still.",
                "",
            )
        }
        ("side", "Inside") => (
            "Follow the inner walls and island boundaries of a selected recessed planar floor. Offset the cutter center into the pocket by its radius and radial allowance. This contour operation follows boundaries; it does not clear the pocket interior.",
            "",
        ),
        ("side", "Outside") => (
            "Follow the model's outer projected silhouette. Offset the center outward by cutter radius and radial allowance, then cut in depth layers. Full-depth reach must fit the saved tool.",
            "",
        ),
        ("side", "On center") => (
            "Record a path centered on the selected chain. This option belongs to the illustrative sample workflow; imported contour generation supports Inside and Outside only.",
            "",
        ),
        ("compensation", "In computer") => (
            "Calculate the tool-radius offset in the CAM preview using the saved diameter. The preview path is the cutter-center path. Changing the diameter requires regeneration.",
            "",
        ),
        ("compensation", "In control") => (
            "Record the intention to apply radius compensation at the controller. Imported generation does not support this mode, and no G41/G42 motion program is emitted.",
            "",
        ),
        ("compensation", "Wear") => (
            "Record a controller wear adjustment in addition to the CAM offset. This sample preference requires a compatible future postprocessor; imported previews currently support In computer only.",
            "",
        ),
        ("compensation", "Inverse wear") => (
            "Record the opposite wear-compensation sign convention. This sample preference is not implemented for imported machining or controller output.",
            "",
        ),
        ("coolant", "Flood") => (
            "Flood coolant: LinuxCNC export writes M8 when the operation starts, and M9 when the coolant changes, at a tool change and at the end of the program. The preview does not simulate coolant.",
            "",
        ),
        ("coolant", "Mist") => (
            "Mist coolant: LinuxCNC export writes M7 when the operation starts, and M9 when the coolant changes, at a tool change and at the end of the program. The preview does not simulate coolant.",
            "",
        ),
        ("coolant", "Air") => (
            "Air blast for chip clearing: LinuxCNC export writes the machine's Air output (M7 or M8, set on the machine) when the operation starts, and M9 when the coolant changes, at a tool change and at the end of the program. Export is refused when the machine has no Air output. The preview does not simulate coolant.",
            "",
        ),
        ("coolant", "Off") => (
            "No coolant: LinuxCNC export writes no coolant command for this operation, after an M9 when an earlier operation left coolant on. The preview does not simulate coolant.",
            "",
        ),
        ("stock_type", "Relative size box") => (
            "Build the blank from the placed part bounds plus six independent offsets. Top adds material only above the part; Right adds material only on +X. Offsets follow setup rotation and translation. The first setup defines the blank for the whole job.",
            "",
        ),
        ("stock_type", "Round bar") => (
            "A round bar along X of the entered diameter and length, centred in X/Y with its bottom at Z=0. Its box is the stock everywhere a box is used; the preview and simulation cut the bar. Hold it in the chuck.",
            "",
        ),
        ("chuck_jaws", "3-jaw") => (
            "A self-centring 3-jaw chuck: it grips round stock. A box is refused in it.",
            "",
        ),
        ("chuck_jaws", "4-jaw") => (
            "A 4-jaw chuck, for a box (jaws square on its sides) or round stock.",
            "",
        ),
        ("chuck_tailstock", "Retracted") => (
            "The tailstock stays on the table with its centre drawn back: its centre, quill and body are checked, and the stock must end short of the centre.",
            "",
        ),
        ("chuck_tailstock", "Supports the stock") => (
            "The tailstock's centre is in the stock's free end: checked as it stands, and the stick-out bends less. The end must be within the quill's travel.",
            "",
        ),
        ("chuck_tailstock", "Off the table") => (
            "The tailstock is taken off the table: it is not checked, and the stick-out bends as a free end.",
            "",
        ),
        ("stock_type", "Fixed size box") => (
            "Use the entered overall X, Y and Z stock dimensions. The preview centers it in X/Y with its bottom at Z=0. The box can be smaller than the part, which is reported. Relative offsets are retained for switching back.",
            "",
        ),
        ("origin", "Model origin") => (
            "Program zero is the imported file's own 0,0,0, placed with this setup's rotation and translation. With the probe, Probe corner on the Control page finds it from the front-left corner of the stock top, set off from it as the program has it. Check that the point can be located on the machine before choosing it.",
            "The LinuxCNC program lists the part's box in work coordinates and, while no earlier setup has cut it, the stock top's Z, to touch off on a part wall or that stock top.",
        ),
        ("origin", "A axis at stock end") => (
            "For work on the rotary A axis: Y0 Z0 on the A centreline, X0 at the stock's free (+X) end face. The Control page's Rotary A calibration measures the centreline and finds A zero on a flat; touch off X0 on the end face. A turns the stock about this zero, so an indexed setup keeps it.",
            "The machine's rotary geometry gives where the centreline is; while it is assumed (not probed) or has gone stale, indexed setups say so.",
        ),
        ("origin", "A axis at chuck face") => (
            "For work on the rotary A axis: Y0 Z0 on the A centreline, X0 on the chuck's face. Needs the stock held in the chuck (Modelled workholding). A turns the stock about this zero, so an indexed setup keeps it.",
            "The face is where the jaws' back is, the stick-out and the jaw length in from the stock's free end; Y0 Z0 and A0 come from the Control page's Rotary A calibration.",
        ),
        ("post_tool_length", "Tool table (G43 H#)") => (
            "Write T# M6, then G43 H# after each tool change. LinuxCNC applies the length stored for that tool number, so measure every tool into its tool table first.",
            "",
        ),
        ("post_tool_length", "Touch off each tool (one program per tool)") => (
            "Write one program per run of the same tool, with no tool change or G43 inside. LinuxCNC's AXIS cannot touch off while a program is paused, so tool changes are not placed inside a program.",
            "Touch the first tool off at the work zero with AXIS Touch Off (not Tool Touch Off) and note the Z of a fixed reference such as a vise jaw; touch each later tool off on that reference. Run G49 in MDI before touching off.",
        ),
        ("post_retract", "Machine Z home (G53 G0 Z0)") => (
            "Retract to machine Z0 with G53 before tool changes and at the end. On a standard LinuxCNC mill Z0 is the top of travel; confirm this on your machine.",
            "",
        ),
        ("post_retract", "Setup clearance plane") => (
            "Retract with G0 Z to the setup's highest clearance height, in work coordinates. Use it when machine Z0 is not the top of travel. There may be less room to change tools.",
            "",
        ),
        ("post_air", "M7") => (
            "Operations set to Air turn on M7, the mist output. Choose this when the air blast is wired to mist.",
            "",
        ),
        ("post_air", "M8") => (
            "Operations set to Air turn on M8, the flood output. Choose this when the air blast is wired to flood.",
            "",
        ),
        ("post_air", "Not available") => (
            "This machine has no air blast output. Export is refused for operations set to Air.",
            "",
        ),
        ("fixture", "None") => (
            "Record no fixture. The setup's header says no workholding is modelled, and the setup sheet and the program header name none.",
            "",
        ),
        ("fixture", "Low profile clamps") => (
            "Record clamps holding the part against a support. Plan access around clamp locations. This choice only describes the setup: to have clamps checked, model them under Modelled workholding.",
            "",
        ),
        ("fixture", "Vise") => (
            "Record vise workholding. Consider jaw contact, exposed height and reclamping. This choice only describes the setup: to have the jaws checked, model the vise under Modelled workholding.",
            "",
        ),
        ("fixture", "Soft jaws") => (
            "Record custom machined jaws that locate this part. Jaw shape and jaw machining are not generated by this selection. This choice only describes the setup: the jaws are not checked, but a vise or clamps modelled under Modelled workholding are.",
            "",
        ),
        ("fixture", "Chuck") => (
            "Record chuck workholding. This choice only describes the setup: to have the chuck checked (its jaws, grip, tailstock, every A angle), choose Chuck under Modelled workholding, on a machine with a rotary A axis.",
            "",
        ),
        ("fixture", "Custom fixture") => (
            "Record a dedicated fixture for the setup. Add locating and clamping instructions in Notes; the fixture's geometry is not modelled. This choice only describes the setup: it is not checked, but a vise or clamps modelled under Modelled workholding are.",
            "",
        ),
        ("reposition", "Initial placement") => (
            "Document the first loading and locating of the blank. Set the part transform and initial stock in the setup fields.",
            "",
        ),
        ("reposition", "Reclamp and probe origin") => (
            "Document a manual flip or reclamping followed by probing. Set the new rotation/translation separately. Earlier removed material is retained across the setup change. Setups indexed on A after it turn the stock in this new clamping.",
            "",
        ),
        ("reposition", "Index rotary axis") => (
            "A turns the stock in the clamping of the setup before it: no re-clamp, no re-probing. This setup's frame is that one's turned about the A centreline by each A angle listed, and it keeps that setup's work zero, offset, machine and chuck.",
            "List one angle (90) or several for the same operations (0, 90, 180, -90). Every operation in the setup is generated at each angle in turn, as 3-axis cuts on the stock and part as A poses them, and between angles the tool retracts above all that A swings (the stock, the part and the jaws, at any angle, plus the operation's clearance), A turns, and the tool comes back down; every operation enters that way, from above all A swings over its first cut, even at the angle A is already at; a retract above the Z travel is refused, never cut short. Every move is checked on the machine, the stock at the centre height: X, Y and Z within the travel, A within the machine's A range and a UC100 profile's soft limits, a cut below the Z travel or a retract above it refused with how far and the stock's diameter, and the tool, its holder and the spindle kept clear of the chuck, its jaws, the rotary head and the tailstock at each angle's cuts and while A turns. Positive A turns the part by the right-hand rule about +X: clockwise seen from the chuck along +X, so a flat on the part's +Y side is on top at A90. Needs a machine with A around X. A work zero on the A axis has Y0 Z0 on the centreline the Control page's Rotary A calibration measures; while it is assumed or stale, the setup says so. LinuxCNC programs turn A with G0 A alone, the tool above all that A swings; a run on the UC100 turns A the same way, alone, and starts only on a measured, current A centreline with the work zero on it (within 0.02 mm).",
        ),
        ("reposition", "Change fixture") => (
            "Document moving the part to different workholding. Set the new placement and locating instructions. Stock remains the result of earlier enabled operations.",
            "",
        ),
        ("axes", "3") => (
            "Save an XYZ machine with three linear axes. The supported imported previews use a fixed tool axis in each setup; manual repositioning is represented by another setup.",
            "",
        ),
        ("axes", "4") => (
            "Save XYZ plus one rotary axis. Select the axis arrangement and indexed/continuous capability next. This profile does not enable a rotary toolpath kernel.",
            "",
        ),
        ("axes", "5") => (
            "Save XYZ plus two rotary axes. Specify both arrangements, limits, directions and pivots. Simultaneous tool-axis planning and inverse kinematics remain pending.",
            "",
        ),
        ("units", "mm") => (
            "Interpret machine travel, pivot lengths and maximum feed in millimeters and mm/min. Project geometry and tool dimensions also use mm. Changing this field changes interpretation; entered numbers are not automatically converted.",
            "",
        ),
        ("units", "inch") => (
            "Interpret machine travel and pivot lengths in inches, and maximum feed in inches/min. The project and tools remain in mm. Existing numeric entries are not automatically converted when switching units.",
            "",
        ),
        ("mode", "3-axis") => (
            "Save fixed-axis XYZ milling capability. Repositioning the part uses another setup; the tool orientation stays fixed during each supported preview.",
            "",
        ),
        ("mode", "Indexed (3+1)") => (
            "Save a single rotary axis that positions before cutting and stays fixed during XYZ motion. Index a setup (Reposition → Index rotary axis) to run its operations at each A angle it lists. The rotary unit's place and chuck and tailstock sizes are saved with the machine (Rotary unit); programs and UC100 runs turn A alone.",
            "",
        ),
        ("mode", "Indexed (3+2)") => (
            "Save two rotary axes that orient the part/tool before fixed-axis XYZ cutting. Automatic indexing and inverse kinematics are not implemented.",
            "",
        ),
        ("mode", "Continuous 4-axis") => (
            "Save simultaneous XYZ and one-rotary capability. Continuous rotary paths and machine simulation are not yet available.",
            "",
        ),
        ("mode", "Simultaneous 5-axis") => (
            "Save coordinated XYZ and two-rotary capability. Tool-axis planning, singularity handling and machine kinematics are not yet available.",
            "",
        ),
        ("direction" | "direction2", "Positive") if matches!(target, Target::Machine) => (
            "Record the positive rotary sign convention for this axis. Match the machine documentation; this setting is saved for future kinematics.",
            "",
        ),
        ("direction" | "direction2", "Negative") if matches!(target, Target::Machine) => (
            "Record the reversed rotary sign convention for this axis. This changes the intended sign mapping, not the current part placement or a live machine command.",
            "",
        ),
        ("tool_change", "Manual") => (
            "Record that an operator exchanges tools. Plan an accessible change position and operator instructions. Preview timing currently omits tool-change time.",
            "",
        ),
        ("tool_change", "Automatic") => (
            "Record an automatic tool changer. Tool-change motions and controller macros are not generated, and their time is not included in the estimate.",
            "",
        ),
        ("tcp", "Yes") => (
            "Record controller tool-center-point compensation capability. A future matching postprocessor and machine kinematic model are required to use it; the current preview does not activate TCP.",
            "",
        ),
        ("tcp", "No") => (
            "Record that controller TCP compensation is unavailable. Future multi-axis output would need the appropriate machine-coordinate calculations. No such output is currently generated.",
            "",
        ),
        ("type", "Flat end mill") => (
            "A cylindrical cutter with a flat bottom. Supported for Facing, Pocket, Contour, Parallel and Waterline within their stated limits. Its radius determines the smallest reachable internal corner.",
            "",
        ),
        ("type", "Ball end mill") => (
            "A cutter with a spherical end. Supported by Parallel finishing and spherical-bottom stock removal. Curved floors follow the ball radius; flat Pocket and Contour previews require a flat end mill.",
            "",
        ),
        ("type", "Bull nose end mill") => (
            "A flat-ended cutter with a corner radius. The profile can be saved, but imported preview kernels do not yet model a bull-nose cutter.",
            "",
        ),
        ("type", "Face mill") => (
            "A wide flat-bottom cutter intended for planar facing. The Facing preview uses its diameter and finite flute length. Holder clearance to modelled vise jaws and clamps is checked; other fixtures still need a future collision model.",
            "",
        ),
        ("type", "Drill") => (
            "A hole-making tool used by Drilling. The preview removes its flutes' cylinder over a 118° point, with the programmed tip at the point. The tool must fit the detected bore.",
            "",
        ),
        ("type", "Chamfer mill") => (
            "A pointed cone of its tip angle. Drilling with it countersinks each detected opening: it plunges until the cone is as wide as the opening at its rim. Chamfers along edges are not generated.",
            "",
        ),
        ("ramp", "Helix") => (
            "Record a circular descending entry. This sample-only preference is not an imported helix generator; imported Pocket offers Profile ramp or Plunge.",
            "",
        ),
        ("ramp", "Zigzag") => (
            "Record a back-and-forth descending entry. This illustrative preference is not implemented in imported Pocket generation.",
            "",
        ),
        ("ramp", "Plunge") => {
            diagram = Some(Diagram::Entry(false));
            (
                "Record a straight vertical entry requiring a plunge-capable cutter. Imported Pocket uses its separate Contour entry setting.",
                "",
            )
        }
        ("ramp", "Profile") => {
            diagram = Some(Diagram::Entry(true));
            (
                "Record a descending entry along a contour. Imported Pocket's implemented Profile ramp is selected in Contour entry.",
                "",
            )
        }
        ("keep_down", "Yes") => (
            "Record a preference for links close to cutting depth. This sample field does not enable collision-aware linking. Imported Facing uses Between facing passes; Pocket retracts between contours.",
            "",
        ),
        ("keep_down", "No") => (
            "Record a preference to lift between separate cuts. Actual imported Facing linking is controlled by Between facing passes; Pocket already retracts between contours.",
            "",
        ),
        ("rest", "Yes") => (
            "Record the intention to machine material remaining from an earlier operation. Fully cleared top layers are skipped automatically. This saved preference does not yet isolate leftover islands or corners; other air cuts can remain.",
            "",
        ),
        ("rest", "No") => (
            "Do not request leftover-region rest machining. Automatic start-height planning and stock inheritance still use preceding cuts; this preference never resets the physical blank.",
            "",
        ),
        ("pattern", "Parallel") => (
            "Record parallel surface passes in the illustrative strategy. For actual imported finishing, use Parallel and set its spacing, direction and slope range.",
            "",
        ),
        ("pattern", "Scallop") => (
            "Record constant surface-spacing finishing. The Scallop machining kernel is unavailable; this sample preference does not generate an imported path.",
            "",
        ),
        ("rotary_axis", "A around X") => (
            "Record rotation about machine X. This is a saved rotary preference; continuous rotary path generation is not connected.",
            "",
        ),
        ("rotary_axis", "B around Y") => (
            "Record rotation about machine Y. This is a saved rotary preference; continuous rotary path generation is not connected.",
            "",
        ),
        ("rotary_axis", "C around Z") => (
            "Record rotation about machine Z. This is a saved rotary preference; continuous rotary path generation is not connected.",
            "",
        ),
        ("rotary_style", "Line") => (
            "Record linear passes along the rotary workpiece. Continuous rotary toolpaths remain unavailable.",
            "",
        ),
        ("rotary_style", "Spiral") => (
            "Record combined axial travel and rotation along a spiral. This is a preference for a future rotary engine, not a calculated machining path.",
            "",
        ),
        ("rotary_style", "Circular") => (
            "Record circular passes around the rotary axis. Actual rotary interpolation and machine simulation remain pending.",
            "",
        ),
        ("axis_control", "Surface normal") => (
            "Record a tool axis perpendicular to the local surface. The sample illustrates this idea; imported 5-axis tool orientation and collision solving are unavailable.",
            "",
        ),
        ("axis_control", "Lead and lean") => (
            "Record forward/backward and sideways tilt relative to the surface path. Lead angle and Lean angle describe those tilts; the actual 5-axis solver is unavailable.",
            "",
        ),
        ("collision", "Retract and relink") => (
            "Record a preference to lift and find another link around an obstruction. No fixture/holder collision solver currently performs this action.",
            "",
        ),
        ("collision", "Tilt within limits") => (
            "Record a preference to change tool tilt around an obstruction, within the saved limits. Collision avoidance and 5-axis inverse kinematics are not implemented.",
            "",
        ),
        ("stock_visibility", "Shown") => (
            "Stock is visible. During simulation this is the remaining material after earlier operations and the current cut. Amber marks original outer faces and green marks cut surfaces. Hide it to inspect the reference part.",
            "",
        ),
        ("stock_visibility", "Hidden") => (
            "Stock is hidden and the reference part is visible. Material removal continues to be calculated; this does not reset stock or change the machining job.",
            "",
        ),
        ("uncut_visibility", "Shown") => (
            "The faces of the part that the stock still covers after every setup are magenta. With the simulated stock shown, they are drawn over it wherever its surface does not hide them. A selected one keeps its magenta and gets a blue outline.",
            "",
        ),
        ("uncut_visibility", "Hidden") => (
            "The part is drawn without marking the faces the stock still covers. The Still uncut list and the share of the surface under the 3D view are unchanged.",
            "",
        ),
        ("path_visibility", "Shown") => (
            "The current operation's preview path is visible. Hide it for an unobstructed view of the machined stock. This display choice does not disable the operation or change material removal.",
            "",
        ),
        ("path_visibility", "Hidden") => (
            "Path lines are hidden so the part and stock are easier to inspect. The operation remains enabled and its simulation is unchanged. Show the path again to inspect cutter travel.",
            "",
        ),
        ("stock_detail", "Draft") => (
            "Use an adaptive octree up to depth 7: a 128-cell span along the longest padded stock dimension. This updates fastest and uses fewer surface triangles. Small curves and thin details may be missed. The continuous cutter history remains unchanged.",
            STOCK_DETAIL_MORE,
        ),
        ("stock_detail", "Standard") => (
            "Use an adaptive octree up to depth 8: a 256-cell span along the padded stock dimension. This balances curve detail and update time. Uniform regions remain coarse; the displayed finest cell size is a sampling scale, not machining tolerance.",
            STOCK_DETAIL_MORE,
        ),
        ("stock_detail", "Fine") => (
            "Use an adaptive octree up to depth 9: a 512-cell span along the padded stock dimension. This resolves smaller curved features with more surface triangles and longer updates. No dense 512³ grid is stored. Features below the finest cell can still be missed.",
            STOCK_DETAIL_MORE,
        ),
        _ => ("", ""),
    };
    let mut body = body.to_string();
    let mut more = more.to_string();
    if key == "facing_milling" && facing_setting(p, "facing_pattern") == "Both ways (zigzag)" {
        body.push_str("\n\nCurrent pass pattern: Both ways. This preference is used only for One way; the current zigzag path remains mixed.");
    }
    if matches!(key, "facing_milling" | "pocket_milling") {
        body.push_str(&format!(
            "\n\nCurrent spindle: {}.",
            facing_setting(p, "spindle_rotation")
        ));
    }
    if body.is_empty() {
        let plain = |text: String| (text, String::new());
        (body, more) = match key {
            "offset" => plain(format!(
                "Use {value} as this setup's work offset. The LinuxCNC program selects it before any motion; touch it off at the setup's work zero."
            )),
            "controller" if value == "UC100" => plain("AXIS CAM drives this machine itself through the UC100 driver: connect, reset and E-stop it on the Control page. G-code export is not offered for it.".to_string()),
            "uc100_connection" if value == "Simulated" => plain("Connect to a simulated UC100 inside AXIS CAM: the driver's controller, planner and checks run, but no hardware is opened and nothing moves. Its saved offsets and position are kept apart from the real machine's.".to_string()),
            "uc100_connection" => plain("Connect to the real UC100 over USB (macOS). The machine starts in E-stop; press Reset on the Control page to enable it.".to_string()),
            "controller" if value == "LinuxCNC" => plain("LinuxCNC has a 3-axis postprocessor, which also indexes A on a machine with A around X (G0 A alone, never during a move). Export checks every setup and replays each program against the simulated moves before writing it.".to_string()),
            "controller" => plain(format!(
                "Save {value} as the intended controller family for planning. Only LinuxCNC output is implemented, so G-code export is refused for this machine."
            )),
            "arrangement" => plain(format!(
                "Save the {value} rotary layout. {} A rotates about X, B about Y, and C about Z. This records the machine configuration, and for an A table around X the rotary unit's place and size under Rotary unit; inverse kinematics are not implemented.",
                if value.contains("head-head") {
                    "Both rotary axes are on the spindle head."
                } else if value.contains("head-table") {
                    "Rotary motion is split between the spindle head and table."
                } else {
                    "Rotary axes are on the table and move the workpiece."
                }
            )),
            "origin" => origin_help(value).unwrap_or_default(),
            "speed" => plain(format!(
                "Play the preview at {value}× the estimated machining rate. This changes playback speed only, not the programmed feed or stock result. Estimates omit acceleration and tool changes."
            )),
            _ => {
                let field = crate::help::field_help(key, label, target);
                (
                    format!("Current saved value: {value}. {}", field.body),
                    field.more,
                )
            }
        };
    }
    // No generic operation animation on a choice: each picture must actually
    // describe this option, rather than silently showing a different mode.
    Help {
        title: format!("{label} · {value}"),
        body,
        more,
        diagram,
    }
}

/// What a stock-based work zero (`post::stock_zero`) is, and under More…
/// how it is touched off and found. None for any other value.
fn origin_help(value: &str) -> Option<(String, String)> {
    use crate::post::At;
    let (xy, top) = crate::post::stock_zero(value)?;
    let face = if top { "top" } else { "bottom" };
    let at = match crate::control::probe_spot(xy).as_str() {
        "its centre" => format!("the centre of the stock's {face}"),
        spot => format!("{spot} of the stock's {face}"),
    };
    let sides = if xy == [At::Mid, At::Mid] {
        "X0 Y0 from the four sides near their middles".to_string()
    } else {
        let side = |k: usize| {
            let axis = ["X", "Y"][k];
            let which = match xy[k] {
                At::Min => format!("the -{axis} side"),
                At::Max => format!("the +{axis} side"),
                At::Mid => format!("the -{axis} and +{axis} sides"),
            };
            let a = 1 - k;
            let near = match xy[a] {
                At::Mid => "near the middle".to_string(),
                end => format!(
                    "near the {}",
                    crate::post::ENDS[a][usize::from(end == At::Max)]
                ),
            };
            format!("{axis}0 from {which} {near}")
        };
        format!("{} and {}", side(0), side(1))
    };
    let z = if top {
        "Touch off Z0 on the stock top there".to_string()
    } else {
        "The bottom sits on the workholding: touch off the stock top over it and enter the stock's height as its Z, which the program's header gives,".to_string()
    };
    Some((
        format!(
            "Program zero is {at}, in setup coordinates, on the stock box this setup starts from: for a later setup the stock the earlier setups leave, turned as this setup turns it. G-code export and a run are refused when an earlier setup has cut the stock where it is touched off."
        ),
        format!(
            "{z} and {sides}, between 1 mm and 10 mm below the top (a quarter of the stock's height when that is less), above the workholding; the program's header gives the figures. {} on the Control page finds exactly this point with the probe. G-code export and a run are refused when an earlier setup has cut the stock where it is touched off, and say which work zeros are still on the stock.",
            crate::control::probe_name(xy)
        ),
    ))
}

pub fn gallery(kind: u8) -> Vec<Help> {
    let options: [(&str, &str, &str); 3] = match kind {
        5 => [
            ("facing_pattern", "Both ways (zigzag)", "CW"),
            ("facing_pattern", "One way", "CW"),
            ("facing_link", "Keep tool down", "CW"),
        ],
        6 => [
            ("pocket_milling", "Climb", "CW"),
            ("pocket_milling", "Conventional", "CW"),
            ("pocket_milling", "Climb", "CCW"),
        ],
        7 => [
            ("pocket_entry", "Profile ramp", "CW"),
            ("pocket_entry", "Plunge", "CW"),
            ("cycle", "Deep drilling", "CW"),
        ],
        8 => [
            ("containment", "Tool inside boundary", "CW"),
            ("containment", "Tool center on boundary", "CW"),
            ("containment", "Tool outside boundary", "CW"),
        ],
        9 => [
            ("stock_type", "Relative size box", "CW"),
            ("stock_type", "Fixed size box", "CW"),
            ("stock_detail", "Fine", "CW"),
        ],
        10 => [
            ("cycle", "Drilling", "CW"),
            ("cycle", "Chip breaking", "CW"),
            ("cycle", "Counterboring", "CW"),
        ],
        _ => [
            ("spindle_rotation", "Clockwise (CW)", "CW"),
            ("spindle_rotation", "Counterclockwise (CCW)", "CCW"),
            ("facing_link", "Retract each pass", "CW"),
        ],
    };
    options
        .into_iter()
        .map(|(key, value, spin)| {
            let mut p = Fields::new();
            p.insert(
                "spindle_rotation".into(),
                if spin == "CW" {
                    "Clockwise (CW)"
                } else {
                    "Counterclockwise (CCW)"
                }
                .into(),
            );
            let label = match key {
                "facing_pattern" => "Pass pattern",
                "facing_link" => "Between passes",
                "pocket_milling" => "Milling direction",
                "pocket_entry" => "Contour entry",
                "cycle" => "Drilling cycle",
                "containment" => "Containment",
                "stock_type" => "Stock mode",
                "stock_detail" => "Stock detail",
                _ => "Spindle rotation",
            };
            selected(key, label, value, Target::Operation, &p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_method_tracks_effective_spindle_and_pattern_including_legacy_defaults() {
        let mut p = Fields::new();
        let legacy = selected(
            "facing_milling",
            "Milling direction",
            facing_setting(&p, "facing_milling"),
            Target::Operation,
            &p,
        );
        assert!(legacy.title.ends_with("Conventional"));
        assert!(legacy.body.contains("current zigzag path remains mixed"));
        assert_eq!(legacy.diagram, Some(Diagram::MillingChoice(false, true)));
        p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        p.insert("facing_pattern".into(), "One way".into());
        let changed = selected(
            "facing_milling",
            "Milling direction",
            "Climb",
            Target::Operation,
            &p,
        );
        assert!(
            changed
                .body
                .contains("Current spindle: Counterclockwise (CCW)")
        );
        assert!(!changed.body.contains("current zigzag path remains mixed"));
        assert_eq!(changed.diagram, Some(Diagram::MillingChoice(true, false)));
    }
    #[test]
    fn switching_options_changes_the_explanation_and_corresponding_illustration() {
        let p = Fields::new();
        for (key, values) in [
            ("facing_pattern", vec!["Both ways (zigzag)", "One way"]),
            ("facing_link", vec!["Keep tool down", "Retract each pass"]),
            ("pocket_entry", vec!["Profile ramp", "Plunge"]),
            (
                "cycle",
                vec![
                    "Drilling",
                    "Chip breaking",
                    "Deep drilling",
                    "Counterboring",
                ],
            ),
            (
                "containment",
                vec![
                    "Tool inside boundary",
                    "Tool center on boundary",
                    "Tool outside boundary",
                ],
            ),
            (
                "spindle_rotation",
                vec!["Clockwise (CW)", "Counterclockwise (CCW)"],
            ),
        ] {
            let cards: Vec<_> = values
                .iter()
                .map(|value| selected(key, key, value, Target::Operation, &p))
                .collect();
            for (i, a) in cards.iter().enumerate() {
                assert!(a.title.contains(values[i]));
                assert!(a.diagram.is_some());
                for b in &cards[i + 1..] {
                    assert_ne!(a.body, b.body);
                    assert_ne!(a.diagram, b.diagram);
                }
            }
        }
    }
    #[test]
    fn choice_help_says_what_the_workholding_check_covers_and_what_it_does_not() {
        let p = Fields::new();
        let body = |key: &str, value: &str, target| selected(key, key, value, target, &p).body;
        // The fixture choices only describe the setup; modelling is elsewhere.
        for value in [
            "Low profile clamps",
            "Vise",
            "Soft jaws",
            "Chuck",
            "Custom fixture",
        ] {
            let text = body("fixture", value, Target::Setup);
            assert!(
                text.contains("only describes the setup") && text.contains("Modelled workholding"),
                "{value}: {text}"
            );
            for stale in ["pending", "not modeled", "does not contain jaw geometry"] {
                assert!(!text.contains(stale), "{value}: {text}");
            }
        }
        assert!(
            body("fixture", "Vise", Target::Setup)
                .contains("model the vise under Modelled workholding")
        );
        assert!(
            body("fixture", "Low profile clamps", Target::Setup)
                .contains("model them under Modelled workholding")
        );
        // The texts that said fixtures are not checked: jaws and clamps are.
        for (key, value, target) in [
            ("facing_link", "Keep tool down", Target::Operation),
            ("facing_link", "Retract each pass", Target::Operation),
        ] {
            let text = body(key, value, target);
            assert!(
                text.contains("a move within the clearance gap of a modelled vise jaw or clamp is")
                    && text.contains("refused at export"),
                "{value}: {text}"
            );
            assert!(!text.contains("fixture clearance is not checked"), "{text}");
        }
        let through = selected("through", "through", "Yes", Target::Operation, &p);
        assert!(
            through
                .body
                .contains("A selected hole that is blind is refused"),
            "{}",
            through.body
        );
        assert!(
            through.more.contains(
                "modelled vise jaws and clamps are checked as boxes with a clearance gap"
            ) && through.more.contains(
                "the material, parallels or the table underneath are not collision checked"
            ),
            "{}",
            through.more
        );
        let face_mill = body("type", "Face mill", Target::Tool);
        assert!(
            face_mill.contains("Holder clearance to modelled vise jaws and clamps is checked"),
            "{face_mill}"
        );
    }
    #[test]
    fn choice_help_keeps_implemented_and_saved_preferences_distinct() {
        let p = Fields::new();
        let rest = selected("rest", "Rest machining", "Yes", Target::Operation, &p);
        assert!(
            rest.body
                .contains("Fully cleared top layers are skipped automatically")
        );
        assert!(
            rest.body
                .contains("does not yet isolate leftover islands or corners")
        );
        let five = selected(
            "mode",
            "Capability",
            "Simultaneous 5-axis",
            Target::Machine,
            &p,
        );
        assert!(five.body.contains("not yet available"));
        let standard = selected("stock_detail", "Detail", "Standard", Target::Operation, &p);
        let fine = selected("stock_detail", "Detail", "Fine", Target::Operation, &p);
        assert!(standard.body.contains("256"));
        assert!(fine.body.contains("512"));
        assert_ne!(standard.body, fine.body);
        let relative = selected(
            "stock_type",
            "Stock mode",
            "Relative size box",
            Target::Setup,
            &p,
        );
        assert!(relative.body.contains("six independent offsets"));
        let fixed = selected(
            "stock_type",
            "Stock mode",
            "Fixed size box",
            Target::Setup,
            &p,
        );
        assert!(fixed.body.contains("overall X, Y and Z"));
        assert!(fixed.diagram.is_none());
    }
    #[test]
    fn coolant_help_says_what_the_export_writes() {
        let p = Fields::new();
        let body = |v: &str| selected("coolant", "Coolant", v, Target::Operation, &p).body;
        assert!(body("Flood").contains("M8"));
        assert!(body("Mist").contains("M7"));
        assert!(body("Air").contains("machine's Air output"));
        assert!(body("Air").contains("refused"));
        assert!(body("Off").contains("no coolant command"));
        let field = crate::help::field_help("coolant", "Coolant", Target::Operation).body;
        assert!(field.contains("M8") && field.contains("M7") && field.contains("M9"));
        for text in ["Flood", "Mist", "Air", "Off"]
            .map(body)
            .into_iter()
            .chain([field])
        {
            assert!(text.contains("does not simulate coolant"), "{text}");
            for stale in [
                "future postprocessing",
                "no machine output",
                "saved preference",
            ] {
                assert!(!text.contains(stale), "{text}");
            }
        }
        let rpm = crate::help::field_help("rpm", "Spindle speed", Target::Operation).body;
        assert!(rpm.contains("LinuxCNC export writes it as S"), "{rpm}");
    }
    #[test]
    fn post_settings_have_choice_specific_help() {
        let p = Fields::new();
        let cards: Vec<_> = [
            ("post_tool_length", "Tool table (G43 H#)"),
            (
                "post_tool_length",
                "Touch off each tool (one program per tool)",
            ),
            ("post_retract", "Machine Z home (G53 G0 Z0)"),
            ("post_retract", "Setup clearance plane"),
            ("post_air", "M7"),
            ("post_air", "M8"),
            ("post_air", "Not available"),
        ]
        .iter()
        .map(|(k, v)| selected(k, "Post", v, Target::Machine, &p))
        .collect();
        for (i, a) in cards.iter().enumerate() {
            assert!(!a.body.starts_with("Current saved value"));
            for b in &cards[i + 1..] {
                assert_ne!(a.body, b.body);
            }
        }
        assert!(
            selected("controller", "Controller", "LinuxCNC", Target::Machine, &p)
                .body
                .contains("3-axis postprocessor")
        );
        assert!(
            selected("controller", "Controller", "GRBL", Target::Machine, &p)
                .body
                .contains("refused")
        );
        assert!(
            selected("controller", "Controller", "UC100", Target::Machine, &p)
                .body
                .contains("Control page")
        );
        assert!(
            selected(
                "uc100_connection",
                "Connection",
                "Simulated",
                Target::Machine,
                &p
            )
            .body
            .contains("no hardware")
        );
        assert!(
            selected("uc100_connection", "Connection", "USB", Target::Machine, &p)
                .body
                .contains("E-stop")
        );
        for origin in ["Stock top center", "Stock top front-left", "Model origin"] {
            let card = selected("origin", "Origin", origin, Target::Setup, &p);
            assert!(!card.body.contains("pending") && !card.more.contains("pending"));
            assert!(card.body.starts_with("Program zero is "), "{}", card.body);
            assert!(
                card.body
                    .contains("refused when an earlier setup has cut the stock")
                    || origin == "Model origin",
                "{}",
                card.body
            );
            let both = format!("{} {}", card.body, card.more);
            assert!(both.contains("on the Control page finds"), "{both}");
        }
        assert!(
            !crate::help::field_help("number", "Tool number", Target::Tool)
                .body
                .starts_with("Saved parameter")
        );
        for key in ["post_blend", "post_dwell", "uc100_profile"] {
            assert!(
                !crate::help::field_help(key, "Post", Target::Machine)
                    .body
                    .starts_with("Saved parameter")
            );
        }
    }
}
