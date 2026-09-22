// STEP -> bounded binary STL preview. The original STEP remains the CAD source.
#include <STEPControl_Reader.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <TopoDS.hxx>
#include <TopExp_Explorer.hxx>
#include <Poly_Triangulation.hxx>
#include <Standard_Failure.hxx>
#include <fstream>
#include <iostream>
#include <vector>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <stdexcept>

static void u32(std::ostream& out, uint32_t n) {
    for (int i=0; i<4; ++i) out.put((n >> (8*i)) & 255);
}
static void f32(std::ostream& out, float f) {
    uint32_t n; std::memcpy(&n, &f, 4); u32(out, n);
}
int main(int argc, char** argv) {
    if (argc != 3) return 2;
    try {
        STEPControl_Reader reader;
        if (reader.ReadFile(argv[1]) != IFSelect_RetDone)
            throw std::runtime_error("STEP reader could not read the file.");
        reader.SetSystemLengthUnit(1.0); // OCCT system length: millimeters.
        const int roots = reader.NbRootsForTransfer();
        if (roots < 1 || reader.TransferRoots() != roots)
            throw std::runtime_error("Not every STEP root could be translated.");
        TopoDS_Shape shape = reader.OneShape();
        if (shape.IsNull()) throw std::runtime_error("STEP contains no supported shape.");
        BRepMesh_IncrementalMesh mesher(shape, 0.05, false, 0.3, true);
        if (!mesher.IsDone()) throw std::runtime_error("STEP tessellation failed.");
        std::vector<std::array<gp_Pnt, 3>> triangles;
        for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next()) {
            const TopoDS_Face face = TopoDS::Face(ex.Current());
            TopLoc_Location location;
            Handle(Poly_Triangulation) mesh = BRep_Tool::Triangulation(face, location);
            if (mesh.IsNull()) throw std::runtime_error("A STEP face could not be tessellated.");
            if (triangles.size() + mesh->NbTriangles() > 200000)
                throw std::runtime_error("STEP exceeds the 200,000-triangle preview limit.");
            for (int i=1; i<=mesh->NbTriangles(); ++i) {
                int a,b,c; mesh->Triangle(i).Get(a,b,c);
                if (face.Orientation() == TopAbs_REVERSED) std::swap(b,c);
                std::array<gp_Pnt,3> t = {mesh->Node(a),mesh->Node(b),mesh->Node(c)};
                for (auto& p : t) {
                    p.Transform(location.Transformation());
                    if (!std::isfinite(p.X()) || !std::isfinite(p.Y()) || !std::isfinite(p.Z()))
                        throw std::runtime_error("Non-finite STEP coordinate.");
                }
                triangles.push_back(t);
            }
        }
        if (triangles.empty()) throw std::runtime_error("STEP has no surface triangles (wire-only models are unsupported).");
        std::ofstream out(argv[2], std::ios::binary);
        char header[80] = "AXIS OCCT STEP preview; units=mm; deflection=0.05mm";
        out.write(header,80); u32(out,triangles.size());
        for (const auto& t : triangles) {
            f32(out,0); f32(out,0); f32(out,0);
            for (auto p : t) { f32(out,p.X()); f32(out,p.Y()); f32(out,p.Z()); }
            out.put(0); out.put(0);
        }
        if (!out) throw std::runtime_error("Could not write the STEP preview.");
        return 0;
    } catch (const Standard_Failure& e) {
        std::cerr << "Open CASCADE: " << e.GetMessageString() << '\n';
    } catch (const std::exception& e) {
        std::cerr << e.what() << '\n';
    }
    return 1;
}
