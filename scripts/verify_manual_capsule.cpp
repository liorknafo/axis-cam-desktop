// Independent OCCT acceptance: intersect complete nominal/actual capsule
// prisms with the original STEP B-rep, without using the app's mesh tests.
// Build/run commands and numerical limits are in docs/tasks/manual-mill-capsule-slots.md.
#include <STEPControl_Reader.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <gp_Ax2.hxx>
#include <gp_Vec.hxx>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <cmath>
#include <stdexcept>

static double volume(const TopoDS_Shape& s) {GProp_GProps p; BRepGProp::VolumeProperties(s,p);return std::abs(p.Mass());}
static TopoDS_Shape capsule(double ax,double ay,double bx,double by,double r,double low,double high) {
  auto ca=BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(ax,ay,low),gp_Dir(0,0,1)),r,high-low).Shape();
  auto cb=BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(bx,by,low),gp_Dir(0,0,1)),r,high-low).Shape();
  double len=std::hypot(bx-ax,by-ay),nx=-(by-ay)/len*r,ny=(bx-ax)/len*r;
  BRepBuilderAPI_MakePolygon wire;
  wire.Add(gp_Pnt(ax+nx,ay+ny,low));wire.Add(gp_Pnt(bx+nx,by+ny,low));
  wire.Add(gp_Pnt(bx-nx,by-ny,low));wire.Add(gp_Pnt(ax-nx,ay-ny,low));wire.Close();
  auto box=BRepPrimAPI_MakePrism(BRepBuilderAPI_MakeFace(wire.Wire()).Face(),gp_Vec(0,0,high-low)).Shape();
  return BRepAlgoAPI_Fuse(BRepAlgoAPI_Fuse(ca,box).Shape(),cb).Shape();
}
int main(int argc,char**argv) {
 try {
  if(argc!=3) throw std::runtime_error("usage: verify_manual_capsule private.step sweeps.tsv");
  STEPControl_Reader reader;if(reader.ReadFile(argv[1])!=IFSelect_RetDone)throw std::runtime_error("STEP read failed");
  reader.TransferRoots();auto part=reader.OneShape();if(!BRepCheck_Analyzer(part).IsValid())throw std::runtime_error("Invalid STEP B-rep");
  std::ifstream in(argv[2]);double ax,ay,bx,by,r,low,high;int n=0;double total=0.;
  std::cout<<std::setprecision(12)<<"part_volume_mm3="<<volume(part)<<"\n";
  while(in>>ax>>ay>>bx>>by>>r>>low>>high) {
    auto swept=capsule(ax,ay,bx,by,r,low,high);
    if(!BRepCheck_Analyzer(swept).IsValid())throw std::runtime_error("Invalid capsule prism");
    double gouge=volume(BRepAlgoAPI_Common(part,swept).Shape());total+=gouge;
    const double radial_tolerance=.00003, axial_tolerance=.0001;
    auto snap=[](double v){return std::round(v*100.)/100.;};
    double nominal=volume(BRepAlgoAPI_Common(part,capsule(snap(ax),snap(ay),snap(bx),snap(by),r,snap(low),high)).Shape());
    double bounded=volume(BRepAlgoAPI_Common(part,capsule(ax,ay,bx,by,r-radial_tolerance,low+axial_tolerance,high)).Shape());
    double coordinate_error=std::max(std::hypot(ax-snap(ax),ay-snap(ay)),std::hypot(bx-snap(bx),by-snap(by)));
    // Positive controls: small end shift and oversize must intersect the B-rep.
    double shifted=volume(BRepAlgoAPI_Common(part,capsule(ax-.001,ay,bx-.001,by,r,low,high)).Shape());
    double oversized=volume(BRepAlgoAPI_Common(part,capsule(ax,ay,bx,by,r+.001,low,high)).Shape());
    double area=M_PI*r*r+2.*r*std::hypot(bx-ax,by-ay);
    std::cout<<"slot="<<++n<<" area_mm2="<<area<<" common_part_mm3="<<gouge<<" nominal_DRO_common_mm3="<<nominal<<" inset_0.00003_common_mm3="<<bounded<<" max_XY_roundoff_mm="<<coordinate_error<<" shifted_0.001_common_mm3="<<shifted<<" oversized_radius_0.001_common_mm3="<<oversized<<"\n";
    if(nominal>1e-7 || bounded>1e-7 || coordinate_error>radial_tolerance || std::abs(low-snap(low))>axial_tolerance || shifted<0.001 || oversized<0.001)throw std::runtime_error("B-rep no-gouge or positive-control failure");
  }
  if(n!=8)throw std::runtime_error("Expected exactly eight generated sweeps");
  std::cout<<"sum_common_part_mm3="<<total<<"; nominal and tolerance-inset B-rep intersections <=1e-7 mm3; radial envelope 0.00003 mm, axial 0.0001 mm\n";
  return 0;
 }catch(const Standard_Failure&e){std::cerr<<e.GetMessageString()<<"\n";}catch(const std::exception&e){std::cerr<<e.what()<<"\n";}return 1;
}
