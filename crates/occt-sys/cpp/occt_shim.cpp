// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The C ABI wall between Rust and OpenCascade.
//
// Rules for every function in this file:
// - It is `extern "C"` and takes only plain C types.
// - It catches every C++ exception. Nothing may unwind into Rust.
// - It reports failure as a status code plus a NUL-terminated message.
// - It never writes past `err_len` bytes of `err`.
// - Memory it hands out is freed by a matching `cndt_occt_free_*` function.
//
// OpenCascade's data exchange code is not thread-safe; the Rust side holds a
// process-wide lock around every call (see src/lib.rs).

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS_Vertex.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <gp_Ax1.hxx>
#include <GProp_GProps.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Wire.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Vec.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <Poly_Triangulation.hxx>
#include <STEPControl_Reader.hxx>
#include <ShapeAnalysis.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Version.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <exception>
#include <memory>
#include <mutex>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

namespace {

// Status codes. Keep in step with `status` in src/lib.rs.
constexpr int kOk = 0;
constexpr int kReadFailed = 1;
constexpr int kNoShapes = 2;
constexpr int kKernelError = 3;
constexpr int kBadArgument = 4;
constexpr int kMeshFailed = 5;
constexpr int kModelFailed = 6;

void set_error(char* err, std::size_t err_len, const char* message) {
    if (err == nullptr || err_len == 0) {
        return;
    }
    if (message == nullptr) {
        message = "unknown error";
    }
    std::strncpy(err, message, err_len - 1);
    err[err_len - 1] = '\0';
}

// OpenCascade prints warnings and errors to the console by default. A library
// must not write to the terminal behind the app's back, so the console
// printer is removed once. Failures still reach Rust as status codes.
void silence_console_output() {
    static std::once_flag once;
    std::call_once(once, [] {
        Message::DefaultMessenger()->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    });
}

std::uint32_t count_unique(const TopoDS_Shape& shape, TopAbs_ShapeEnum kind) {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, kind, map);
    return static_cast<std::uint32_t>(map.Extent());
}

// Reads a STEP file into `shape`, in millimetres. Returns a status code.
// May throw; callers catch.
int read_step_shape(const char* path, TopoDS_Shape& shape, char* err, std::size_t err_len) {
    silence_console_output();
    STEPControl_Reader reader;
    // The core works in millimetres (architecture rule 10). Set after the
    // reader exists, because creating it registers this parameter.
    Interface_Static::SetCVal("xstep.cascade.unit", "MM");

    if (reader.ReadFile(path) != IFSelect_RetDone) {
        set_error(err, err_len, "the file could not be read as STEP");
        return kReadFailed;
    }
    reader.TransferRoots();
    if (reader.NbShapes() == 0) {
        set_error(err, err_len, "the file contains no shapes");
        return kNoShapes;
    }
    shape = reader.OneShape();
    return kOk;
}


}  // namespace

extern "C" {
// Owns a mesh's arrays. Opaque to Rust.
struct cndt_mesh {
    std::vector<float> positions;          // x, y, z per vertex
    std::vector<std::uint32_t> indices;    // three per triangle
    std::vector<std::uint32_t> triangle_faces;  // face index per triangle
    std::uint32_t face_count = 0;
};

// Read-only view of a mesh. Layout must match `RawMeshView` in src/lib.rs.
struct cndt_mesh_view {
    const float* positions;
    std::size_t positions_len;
    const std::uint32_t* indices;
    std::size_t indices_len;
    const std::uint32_t* triangle_faces;
    std::size_t triangle_faces_len;
    std::uint32_t face_count;
};
}

namespace {

// Triangulates every face of `shape` for display. May throw; callers catch.
int mesh_shape(
    const TopoDS_Shape& shape,
    double deflection,
    cndt_mesh** handle,
    cndt_mesh_view* view,
    char* err,
    std::size_t err_len) {
        // Sequential meshing keeps the result identical on every run.
        BRepMesh_IncrementalMesh mesher(shape, deflection, Standard_False, 0.35, Standard_False);
        if (!mesher.IsDone()) {
            set_error(err, err_len, "the shape could not be triangulated");
            return kMeshFailed;
        }

        // Owned here until success, so an exception cannot leak it.
        auto mesh = std::make_unique<cndt_mesh>();
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(shape, TopAbs_FACE, faces);
        mesh->face_count = static_cast<std::uint32_t>(faces.Extent());

        for (int face_index = 1; face_index <= faces.Extent(); ++face_index) {
            const TopoDS_Face& face = TopoDS::Face(faces(face_index));
            TopLoc_Location location;
            const Handle(Poly_Triangulation) triangulation = BRep_Tool::Triangulation(face, location);
            if (triangulation.IsNull()) {
                continue;  // A face without triangles is skipped, not fatal.
            }
            const gp_Trsf transform = location.Transformation();
            const auto base = static_cast<std::uint32_t>(mesh->positions.size() / 3);
            for (int node = 1; node <= triangulation->NbNodes(); ++node) {
                const gp_Pnt point = triangulation->Node(node).Transformed(transform);
                mesh->positions.push_back(static_cast<float>(point.X()));
                mesh->positions.push_back(static_cast<float>(point.Y()));
                mesh->positions.push_back(static_cast<float>(point.Z()));
            }
            // A reversed face lists its triangles the other way round; swap
            // two corners so every triangle faces outwards.
            const bool reversed = face.Orientation() == TopAbs_REVERSED;
            for (int triangle = 1; triangle <= triangulation->NbTriangles(); ++triangle) {
                int a = 0;
                int b = 0;
                int c = 0;
                triangulation->Triangle(triangle).Get(a, b, c);
                if (reversed) {
                    std::swap(b, c);
                }
                mesh->indices.push_back(base + static_cast<std::uint32_t>(a - 1));
                mesh->indices.push_back(base + static_cast<std::uint32_t>(b - 1));
                mesh->indices.push_back(base + static_cast<std::uint32_t>(c - 1));
                mesh->triangle_faces.push_back(static_cast<std::uint32_t>(face_index - 1));
            }
        }

        view->positions = mesh->positions.data();
        view->positions_len = mesh->positions.size();
        view->indices = mesh->indices.data();
        view->indices_len = mesh->indices.size();
        view->triangle_faces = mesh->triangle_faces.data();
        view->triangle_faces_len = mesh->triangle_faces.size();
        view->face_count = mesh->face_count;
        *handle = mesh.release();
        return kOk;
}
}  // namespace

extern "C" {

// Layout must match `RawSummary` in src/lib.rs.
struct cndt_shape_summary {
    std::uint32_t solids;
    std::uint32_t shells;
    std::uint32_t faces;
    std::uint32_t edges;
    std::uint32_t vertices;
    double min[3];
    double max[3];
    std::uint8_t has_bounds;
};

const char* cndt_occt_version(void) {
    return OCC_VERSION_COMPLETE;
}

int cndt_occt_read_step(const char* path, cndt_shape_summary* out, char* err, std::size_t err_len) {
    if (path == nullptr || out == nullptr) {
        set_error(err, err_len, "null argument");
        return kBadArgument;
    }
    try {
        TopoDS_Shape shape;
        const int status = read_step_shape(path, shape, err, err_len);
        if (status != kOk) {
            return status;
        }

        out->solids = count_unique(shape, TopAbs_SOLID);
        out->shells = count_unique(shape, TopAbs_SHELL);
        out->faces = count_unique(shape, TopAbs_FACE);
        out->edges = count_unique(shape, TopAbs_EDGE);
        out->vertices = count_unique(shape, TopAbs_VERTEX);

        Bnd_Box box;
        BRepBndLib::AddOptimal(shape, box, Standard_False, Standard_False);
        if (box.IsVoid()) {
            out->has_bounds = 0;
        } else {
            box.Get(out->min[0], out->min[1], out->min[2], out->max[0], out->max[1], out->max[2]);
            out->has_bounds = 1;
        }
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (const std::exception& exception) {
        set_error(err, err_len, exception.what());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

// Reads a STEP file and triangulates every face for display.
//
// `deflection` is the largest allowed distance in millimetres between the
// triangles and the true surface. On success `*handle` owns the mesh and
// `*view` points into it; free it with `cndt_occt_free_mesh`. Faces are
// numbered in OpenCascade's deterministic map order, and each face has its
// own vertices, so normals stay sharp across face edges.
int cndt_occt_mesh_step(
    const char* path,
    double deflection,
    cndt_mesh** handle,
    cndt_mesh_view* view,
    char* err,
    std::size_t err_len) {
    if (path == nullptr || handle == nullptr || view == nullptr || !(deflection > 0.0)) {
        set_error(err, err_len, "bad argument");
        return kBadArgument;
    }
    *handle = nullptr;
    try {
        TopoDS_Shape shape;
        const int status = read_step_shape(path, shape, err, err_len);
        if (status != kOk) {
            return status;
        }

        return mesh_shape(shape, deflection, handle, view, err, err_len);
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (const std::exception& exception) {
        set_error(err, err_len, exception.what());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

void cndt_occt_free_mesh(cndt_mesh* handle) {
    delete handle;
}

// ---- Solid modelling ----
//
// Shapes cross to Rust as owned `cndt_shape` handles. Every operation also
// returns its history as (source, face) pairs, so the Rust side can name each
// face by where it came from (decision 0012). Face numbers are positions in
// OpenCascade's face map of the result, which is deterministic.

// An owned shape. Opaque to Rust; free with `cndt_occt_free_shape`.
struct cndt_shape {
    TopoDS_Shape shape;
};

// Owned history pairs. Opaque to Rust; free with `cndt_occt_free_history`.
struct cndt_history {
    std::vector<std::uint32_t> pairs;  // source, face, source, face, ...
};

// One profile curve in model coordinates. Layout must match `RawCurve` in
// src/lib.rs.
//  kind 0: line from p0 to p1
//  kind 1: arc around p0, counter-clockwise about the normal, from p1 to p2
//  kind 2: full circle around p0 with `radius`
struct cndt_curve {
    std::int32_t kind;
    double p0[3];
    double p1[3];
    double p2[3];
    double radius;
};

// Volume and bounds of a shape. Layout must match `RawProperties`.
struct cndt_properties {
    double volume;
    double min[3];
    double max[3];
    std::uint32_t faces;
    std::uint32_t solids;
};

void cndt_occt_free_shape(cndt_shape* shape) {
    delete shape;
}

void cndt_occt_free_history(cndt_history* history) {
    delete history;
}

void cndt_occt_history_view(const cndt_history* history, const std::uint32_t** pairs, std::size_t* len) {
    if (history == nullptr || pairs == nullptr || len == nullptr) {
        return;
    }
    *pairs = history->pairs.data();
    *len = history->pairs.size();
}

}  // extern "C"

namespace {

gp_Pnt point(const double p[3]) {
    return gp_Pnt(p[0], p[1], p[2]);
}

TopoDS_Edge make_edge(const cndt_curve& curve, const gp_Dir& normal) {
    switch (curve.kind) {
        case 0:
            return BRepBuilderAPI_MakeEdge(point(curve.p0), point(curve.p1)).Edge();
        case 1: {
            const gp_Pnt center = point(curve.p0);
            const gp_Pnt start = point(curve.p1);
            const gp_Vec x(center, start);
            const gp_Circ circle(gp_Ax2(center, normal, gp_Dir(x)), x.Magnitude());
            return BRepBuilderAPI_MakeEdge(circle, start, point(curve.p2)).Edge();
        }
        case 2:
            return BRepBuilderAPI_MakeEdge(gp_Circ(gp_Ax2(point(curve.p0), normal), curve.radius)).Edge();
        default:
            throw Standard_Failure("unknown curve kind");
    }
}

// The face map index (0-based) of each face, for history lookups.
std::vector<std::uint32_t> faces_of(const TopTools_ListOfShape& shapes, const TopTools_IndexedMapOfShape& map) {
    std::vector<std::uint32_t> out;
    for (const TopoDS_Shape& shape : shapes) {
        const int index = map.FindIndex(shape);
        if (index > 0) {
            out.push_back(static_cast<std::uint32_t>(index - 1));
        }
    }
    return out;
}

bool is_valid_solid(const TopoDS_Shape& shape, char* err, std::size_t err_len) {
    if (shape.IsNull() || count_unique(shape, TopAbs_SOLID) == 0) {
        set_error(err, err_len, "the operation produced no solid");
        return false;
    }
    BRepCheck_Analyzer check(shape);
    if (!check.IsValid()) {
        set_error(err, err_len, "the operation produced an invalid solid");
        return false;
    }
    return true;
}

}  // namespace

extern "C" {

// Sweeps a planar region along its normal. `curves` holds the loops one
// after another; `loop_sizes` says how many curves each has. The first loop
// is the outer boundary; the others are holes. Loops may run either way.
// Curves in a loop are in order, each starting where the last ended.
//
// History sources: curve index `i` for the side face it swept, `n` for the
// start cap and `n + 1` for the end cap, where `n` is the number of curves.
int cndt_occt_extrude(
    const cndt_curve* curves,
    std::size_t curve_count,
    const std::uint32_t* loop_sizes,
    std::size_t loop_count,
    const double normal_in[3],
    double distance,
    cndt_shape** out,
    cndt_history** history,
    char* err,
    std::size_t err_len) {
    if (curves == nullptr || loop_sizes == nullptr || normal_in == nullptr || out == nullptr ||
        history == nullptr || loop_count == 0 || !(distance != 0.0)) {
        set_error(err, err_len, "bad argument");
        return kBadArgument;
    }
    *out = nullptr;
    *history = nullptr;
    try {
        silence_console_output();
        const gp_Dir normal(normal_in[0], normal_in[1], normal_in[2]);
        std::vector<TopoDS_Edge> edges(curve_count);
        std::vector<TopoDS_Wire> wires;
        std::size_t next = 0;
        for (std::size_t loop = 0; loop < loop_count; ++loop) {
            BRepBuilderAPI_MakeWire wire;
            for (std::uint32_t k = 0; k < loop_sizes[loop]; ++k, ++next) {
                if (next >= curve_count) {
                    set_error(err, err_len, "loop sizes do not match the curves");
                    return kBadArgument;
                }
                wire.Add(make_edge(curves[next], normal));
                if (!wire.IsDone()) {
                    set_error(err, err_len, "a profile loop is not connected");
                    return kModelFailed;
                }
                // The wire may copy an edge to join it; keep the copy, which
                // is the one the face and the sweep will use.
                edges[next] = wire.Edge();
            }
            if (!wire.Wire().Closed()) {
                set_error(err, err_len, "a profile loop is not closed");
                return kModelFailed;
            }
            wires.push_back(wire.Wire());
        }
        const gp_Pln plane(point(curves[0].kind == 2 ? curves[0].p0 : curves[0].p1), normal);
        // Orient the loops whichever way they were drawn: the outer boundary
        // runs counter-clockwise about the normal, holes clockwise. Reversing
        // a wire keeps its edges, so the history below still finds them.
        for (std::size_t i = 0; i < wires.size(); ++i) {
            const TopoDS_Face alone = BRepBuilderAPI_MakeFace(plane, wires[i], Standard_True).Face();
            const bool outer = ShapeAnalysis::IsOuterBound(alone);
            if (outer != (i == 0)) {
                wires[i].Reverse();
            }
        }
        BRepBuilderAPI_MakeFace face(plane, wires[0], Standard_True);
        for (std::size_t i = 1; i < wires.size(); ++i) {
            face.Add(wires[i]);
        }
        if (!face.IsDone()) {
            set_error(err, err_len, "the profile does not make a face");
            return kModelFailed;
        }
        BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(normal) * distance);
        if (!prism.IsDone()) {
            set_error(err, err_len, "the extrusion failed");
            return kModelFailed;
        }
        const TopoDS_Shape solid = prism.Shape();
        if (!is_valid_solid(solid, err, err_len)) {
            return kModelFailed;
        }
        TopTools_IndexedMapOfShape map;
        TopExp::MapShapes(solid, TopAbs_FACE, map);
        auto record = std::make_unique<cndt_history>();
        for (std::size_t i = 0; i < curve_count; ++i) {
            for (const std::uint32_t f : faces_of(prism.Generated(edges[i]), map)) {
                record->pairs.push_back(static_cast<std::uint32_t>(i));
                record->pairs.push_back(f);
            }
        }
        const int first = map.FindIndex(prism.FirstShape());
        const int last = map.FindIndex(prism.LastShape());
        if (first > 0) {
            record->pairs.push_back(static_cast<std::uint32_t>(curve_count));
            record->pairs.push_back(static_cast<std::uint32_t>(first - 1));
        }
        if (last > 0) {
            record->pairs.push_back(static_cast<std::uint32_t>(curve_count + 1));
            record->pairs.push_back(static_cast<std::uint32_t>(last - 1));
        }
        auto shape = std::make_unique<cndt_shape>();
        shape->shape = solid;
        *out = shape.release();
        *history = record.release();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (const std::exception& exception) {
        set_error(err, err_len, exception.what());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

// Fuses (`op` 0) or cuts (`op` 1) `b` from `a`.
//
// History sources: face `i` of `a` is source `i`; face `j` of `b` is source
// `faces(a) + j`. A face carried over unchanged, or split or trimmed, maps to
// each result face it became; a deleted face maps to nothing.
int cndt_occt_boolean(
    const cndt_shape* a,
    const cndt_shape* b,
    std::int32_t op,
    cndt_shape** out,
    cndt_history** history,
    char* err,
    std::size_t err_len) {
    if (a == nullptr || b == nullptr || out == nullptr || history == nullptr || op < 0 || op > 1) {
        set_error(err, err_len, "bad argument");
        return kBadArgument;
    }
    *out = nullptr;
    *history = nullptr;
    try {
        silence_console_output();
        std::unique_ptr<BRepAlgoAPI_BooleanOperation> algo;
        if (op == 0) {
            algo = std::make_unique<BRepAlgoAPI_Fuse>(a->shape, b->shape);
        } else {
            algo = std::make_unique<BRepAlgoAPI_Cut>(a->shape, b->shape);
        }
        if (algo->HasErrors() || !algo->IsDone()) {
            std::ostringstream report;
            algo->DumpErrors(report);
            const std::string text = report.str();
            set_error(err, err_len, text.empty() ? "the boolean operation failed" : text.c_str());
            return kModelFailed;
        }
        // Merge faces that now lie on one surface, as stacked extrusions do.
        // The history follows the merge.
        algo->SimplifyResult();
        const TopoDS_Shape result = algo->Shape();
        if (!is_valid_solid(result, err, err_len)) {
            if (op == 1 && count_unique(result, TopAbs_SOLID) == 0) {
                set_error(err, err_len, "the cut removes the whole body");
            }
            return kModelFailed;
        }
        TopTools_IndexedMapOfShape map;
        TopExp::MapShapes(result, TopAbs_FACE, map);
        auto record = std::make_unique<cndt_history>();
        std::uint32_t source = 0;
        for (const TopoDS_Shape* input : {&a->shape, &b->shape}) {
            TopTools_IndexedMapOfShape inputs;
            TopExp::MapShapes(*input, TopAbs_FACE, inputs);
            for (int i = 1; i <= inputs.Extent(); ++i, ++source) {
                const TopoDS_Shape& face = inputs(i);
                std::vector<std::uint32_t> targets;
                const int same = map.FindIndex(face);
                if (same > 0) {
                    targets.push_back(static_cast<std::uint32_t>(same - 1));
                } else if (!algo->IsDeleted(face)) {
                    targets = faces_of(algo->Modified(face), map);
                }
                for (const std::uint32_t f : targets) {
                    record->pairs.push_back(source);
                    record->pairs.push_back(f);
                }
            }
        }
        auto shape = std::make_unique<cndt_shape>();
        shape->shape = result;
        *out = shape.release();
        *history = record.release();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (const std::exception& exception) {
        set_error(err, err_len, exception.what());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

int cndt_occt_properties(const cndt_shape* shape, cndt_properties* out, char* err, std::size_t err_len) {
    if (shape == nullptr || out == nullptr) {
        set_error(err, err_len, "bad argument");
        return kBadArgument;
    }
    try {
        GProp_GProps props;
        BRepGProp::VolumeProperties(shape->shape, props);
        out->volume = props.Mass();
        Bnd_Box box;
        BRepBndLib::AddOptimal(shape->shape, box, Standard_False, Standard_False);
        if (!box.IsVoid()) {
            box.Get(out->min[0], out->min[1], out->min[2], out->max[0], out->max[1], out->max[2]);
        }
        out->faces = count_unique(shape->shape, TopAbs_FACE);
        out->solids = count_unique(shape->shape, TopAbs_SOLID);
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

// Triangulates a shape for display, as `cndt_occt_mesh_step` does for a file.
int cndt_occt_mesh_shape(
    const cndt_shape* shape,
    double deflection,
    cndt_mesh** handle,
    cndt_mesh_view* view,
    char* err,
    std::size_t err_len) {
    if (shape == nullptr || handle == nullptr || view == nullptr || !(deflection > 0.0)) {
        set_error(err, err_len, "bad argument");
        return kBadArgument;
    }
    *handle = nullptr;
    try {
        return mesh_shape(shape->shape, deflection, handle, view, err, err_len);
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

// The plane of face `face` (0-based, in the face-map order the history
// uses): a point on it and its outward normal. Fails with kModelFailed if
// the face is not flat.
int cndt_occt_face_plane(
    const cndt_shape* shape,
    std::uint32_t face,
    double point_out[3],
    double normal_out[3],
    char* err,
    std::size_t err_len) {
    if (shape == nullptr || point_out == nullptr || normal_out == nullptr) {
        set_error(err, err_len, "bad argument");
        return kBadArgument;
    }
    try {
        TopTools_IndexedMapOfShape map;
        TopExp::MapShapes(shape->shape, TopAbs_FACE, map);
        const int index = static_cast<int>(face) + 1;
        if (index < 1 || index > map.Extent()) {
            set_error(err, err_len, "no such face");
            return kBadArgument;
        }
        const TopoDS_Face& f = TopoDS::Face(map(index));
        const BRepAdaptor_Surface surface(f);
        if (surface.GetType() != GeomAbs_Plane) {
            set_error(err, err_len, "the face is not flat");
            return kModelFailed;
        }
        const gp_Pln plane = surface.Plane();
        gp_Dir normal = plane.Axis().Direction();
        if (f.Orientation() == TopAbs_REVERSED) {
            normal.Reverse();
        }
        const gp_Pnt origin = plane.Location();
        point_out[0] = origin.X();
        point_out[1] = origin.Y();
        point_out[2] = origin.Z();
        normal_out[0] = normal.X();
        normal_out[1] = normal.Y();
        normal_out[2] = normal.Z();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

// Revolves a closed planar polygon `points` (model coordinates, in order,
// not repeating the first) a full turn about the axis through `axis_origin`
// along `axis_dir`. The polygon must lie in a plane containing the axis and
// may touch the axis along an edge.
//
// History sources: edge `i` (from point `i` to point `i + 1`) for the face
// it swept. Edges on the axis sweep no face.
int cndt_occt_revolve(
    const double* points,
    std::size_t point_count,
    const double axis_origin[3],
    const double axis_dir[3],
    cndt_shape** out,
    cndt_history** history,
    char* err,
    std::size_t err_len) {
    if (points == nullptr || point_count < 3 || axis_origin == nullptr || axis_dir == nullptr ||
        out == nullptr || history == nullptr) {
        set_error(err, err_len, "bad argument");
        return kBadArgument;
    }
    *out = nullptr;
    *history = nullptr;
    try {
        silence_console_output();
        std::vector<TopoDS_Edge> edges(point_count);
        BRepBuilderAPI_MakeWire wire;
        for (std::size_t i = 0; i < point_count; ++i) {
            const std::size_t j = (i + 1) % point_count;
            wire.Add(BRepBuilderAPI_MakeEdge(point(points + 3 * i), point(points + 3 * j)).Edge());
            if (!wire.IsDone()) {
                set_error(err, err_len, "the revolve profile is not a closed polygon");
                return kModelFailed;
            }
            edges[i] = wire.Edge();
        }
        BRepBuilderAPI_MakeFace face(wire.Wire(), Standard_True);
        if (!face.IsDone()) {
            set_error(err, err_len, "the revolve profile is not flat");
            return kModelFailed;
        }
        // Building the wire and face can replace edges with copies, so match
        // the finished face's edges to the segments by their end points.
        for (TopExp_Explorer it(face.Face(), TopAbs_EDGE); it.More(); it.Next()) {
            const TopoDS_Edge& edge = TopoDS::Edge(it.Current());
            TopoDS_Vertex a;
            TopoDS_Vertex b;
            TopExp::Vertices(edge, a, b);
            const gp_Pnt pa = BRep_Tool::Pnt(a);
            const gp_Pnt pb = BRep_Tool::Pnt(b);
            for (std::size_t i = 0; i < point_count; ++i) {
                const gp_Pnt p = point(points + 3 * i);
                const gp_Pnt q = point(points + 3 * ((i + 1) % point_count));
                if ((pa.Distance(p) < 1e-7 && pb.Distance(q) < 1e-7) ||
                    (pa.Distance(q) < 1e-7 && pb.Distance(p) < 1e-7)) {
                    edges[i] = edge;
                }
            }
        }
        const gp_Ax1 axis(point(axis_origin), gp_Dir(axis_dir[0], axis_dir[1], axis_dir[2]));
        BRepPrimAPI_MakeRevol revol(face.Face(), axis);
        if (!revol.IsDone()) {
            set_error(err, err_len, "the revolve failed");
            return kModelFailed;
        }
        const TopoDS_Shape solid = revol.Shape();
        if (!is_valid_solid(solid, err, err_len)) {
            return kModelFailed;
        }
        TopTools_IndexedMapOfShape map;
        TopExp::MapShapes(solid, TopAbs_FACE, map);
        auto record = std::make_unique<cndt_history>();
        std::vector<bool> named(static_cast<std::size_t>(map.Extent()), false);
        for (std::size_t i = 0; i < point_count; ++i) {
            for (const std::uint32_t f : faces_of(revol.Generated(edges[i]), map)) {
                record->pairs.push_back(static_cast<std::uint32_t>(i));
                record->pairs.push_back(f);
                named[f] = true;
            }
        }
        // OpenCascade does not report every swept face (a flat face square
        // to the axis is missed). Each segment's midpoint lies on exactly the
        // face it swept, so find the source of any face left over that way.
        for (int f = 1; f <= map.Extent(); ++f) {
            if (named[static_cast<std::size_t>(f - 1)]) {
                continue;
            }
            for (std::size_t i = 0; i < point_count; ++i) {
                const gp_Pnt p = point(points + 3 * i);
                const gp_Pnt q = point(points + 3 * ((i + 1) % point_count));
                const gp_Pnt mid((p.X() + q.X()) / 2, (p.Y() + q.Y()) / 2, (p.Z() + q.Z()) / 2);
                BRepExtrema_DistShapeShape distance(BRepBuilderAPI_MakeVertex(mid).Vertex(), map(f));
                if (distance.IsDone() && distance.Value() < 1e-7) {
                    record->pairs.push_back(static_cast<std::uint32_t>(i));
                    record->pairs.push_back(static_cast<std::uint32_t>(f - 1));
                    break;
                }
            }
        }
        auto shape = std::make_unique<cndt_shape>();
        shape->shape = solid;
        *out = shape.release();
        *history = record.release();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(err, err_len, failure.GetMessageString());
        return kKernelError;
    } catch (const std::exception& exception) {
        set_error(err, err_len, exception.what());
        return kKernelError;
    } catch (...) {
        set_error(err, err_len, "unknown C++ exception");
        return kKernelError;
    }
}

}  // extern "C"
