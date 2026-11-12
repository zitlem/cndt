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

#include <BRepBndLib.hxx>
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

}  // extern "C"
