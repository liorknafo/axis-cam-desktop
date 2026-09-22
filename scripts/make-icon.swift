import AppKit
let destination = CommandLine.arguments[1]
let image = NSImage(size: NSSize(width:1024,height:1024))
image.lockFocus()
NSColor(red:0.11,green:0.18,blue:0.25,alpha:1).setFill()
NSBezierPath(roundedRect:NSRect(x:40,y:40,width:944,height:944),xRadius:208,yRadius:208).fill()
func line(_ points:[NSPoint],_ color:NSColor,_ width:CGFloat) {let p=NSBezierPath();p.move(to:points[0]);for point in points.dropFirst(){p.line(to:point)};p.lineWidth=width;p.lineJoinStyle = .round;p.lineCapStyle = .round;color.setStroke();p.stroke()}
let blue=NSColor(red:0.26,green:0.64,blue:0.93,alpha:1)
let teal=NSColor(red:0.28,green:0.79,blue:0.73,alpha:1)
line([NSPoint(x:210,y:380),NSPoint(x:510,y:220),NSPoint(x:812,y:385),NSPoint(x:510,y:540),NSPoint(x:210,y:380)],blue,24)
line([NSPoint(x:210,y:380),NSPoint(x:210,y:288),NSPoint(x:510,y:128),NSPoint(x:812,y:293),NSPoint(x:812,y:385)],NSColor(red:0.44,green:0.58,blue:0.69,alpha:1),20)
line([NSPoint(x:330,y:380),NSPoint(x:510,y:283),NSPoint(x:690,y:380),NSPoint(x:510,y:475),NSPoint(x:330,y:380)],teal,17)
line([NSPoint(x:510,y:220),NSPoint(x:510,y:128)],blue,20)
let attrs:[NSAttributedString.Key:Any]=[.font:NSFont.systemFont(ofSize:208,weight:.medium),.foregroundColor:NSColor.white,.kern:10]
("AXIS" as NSString).draw(at:NSPoint(x:206,y:619),withAttributes:attrs)
let gold=NSColor(red:0.91,green:0.72,blue:0.35,alpha:1)
line([NSPoint(x:725,y:644),NSPoint(x:725,y:452)],gold,40)
line([NSPoint(x:696,y:450),NSPoint(x:754,y:450)],gold,30)
image.unlockFocus()
let rep=NSBitmapImageRep(data:image.tiffRepresentation!)!
try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:destination))
