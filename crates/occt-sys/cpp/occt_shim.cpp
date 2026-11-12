// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The C ABI wall between Rust and OpenCascade.
//
// Rules for every function in this file:
// - It is `extern "C"` and takes only plain C types.
// - It catches every C++ exception. Nothing may unwind into Rust.
// - It reports failure as a status code plus a NUL-terminated message.
// - It never writes past `err_len` bytes of `err`.

#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <STEPControl_Reader.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Version.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <exception>
#include <mutex>

namespace {

// Status codes. Keep in step with `Status` in src/lib.rs.
constexpr int kOk = 0;
constexpr int kReadFailed = 1;
constexpr int kNoShapes = 2;
constexpr int kKernelError = 3;
constexpr int kBadArgument = 4;

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
        const TopoDS_Shape shape = reader.OneShape();

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

}  // extern "C"
