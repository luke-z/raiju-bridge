// GPL-3.0-or-later. Isolated UMDF2 VHF source; no physical-device filter.
#define WIN32_NO_STATUS
#include <windows.h>
#undef WIN32_NO_STATUS
#include <ntstatus.h>
#include <wdf.h>
#include <vhf.h>
#include <cstring>
#include "../shared.h"

struct DeviceContext {
    SRWLOCK lifetime, state;
    WDFIOTARGET target;
    VHFHANDLE vhf;
    WDFFILEOBJECT owner;
    HANDLE target_file;
    bool powered;
    CreateRequest config;
    DriverStatus status;
    uint8_t input[kMaxReport];
    uint8_t command[2];
    ULONGLONG last_input;
};
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(DeviceContext, context_of);
EVT_WDF_DRIVER_DEVICE_ADD device_add;
EVT_WDF_DEVICE_D0_ENTRY d0_entry;
EVT_WDF_DEVICE_D0_EXIT d0_exit;
EVT_WDF_FILE_CLEANUP file_cleanup;
EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL device_control;
EVT_WDF_TIMER watchdog;
EVT_VHF_ASYNC_OPERATION get_feature, set_feature, get_input, write_output;
extern "C" DRIVER_INITIALIZE DriverEntry;

// VHF can invoke callbacks synchronously: state never spans a VHF call.
// lifetime serializes creation/submission/deletion, power and file cleanup.
// VHF callbacks take only state, and VhfDelete drains them before reuse.
static void destroy_locked(DeviceContext* c) {
    VHFHANDLE handle = c->vhf;
    c->vhf = nullptr;
    c->owner = nullptr;
    AcquireSRWLockExclusive(&c->state);
    c->status.active = 0;
    ReleaseSRWLockExclusive(&c->state);
    if (handle) VhfDelete(handle, TRUE);
}

static void neutral_locked(DeviceContext* c) {
    std::memset(c->input, 0, sizeof(c->input));
    c->input[0] = 1;
    std::memset(c->input + 1, 128, 4);
    c->input[8] = 8;
    c->input[32] = 0x80; c->input[36] = 0x81;
}

static NTSTATUS submit_locked(DeviceContext* c, const uint8_t* data) {
    uint8_t copy[kMaxReport]{};
    const auto length = c->config.input_length;
    std::memcpy(copy, data, length);
    HID_XFER_PACKET packet{copy, length, copy[0]};
    const NTSTATUS status = VhfReadReportSubmit(c->vhf, &packet);
    AcquireSRWLockExclusive(&c->state);
    c->status.last_submit_status = static_cast<uint32_t>(status);
    if (NT_SUCCESS(status)) ++c->status.submitted;
    ReleaseSRWLockExclusive(&c->state);
    return status;
}

static NTSTATUS create_locked(DeviceContext* c, WDFFILEOBJECT owner, const CreateRequest* r) {
    if (!c->powered || !c->target_file) return STATUS_DEVICE_NOT_READY;
    if (c->vhf) return STATUS_DEVICE_BUSY;
    if (!owner || r->version != 1 || r->reserved ||
        !r->descriptor_length || r->descriptor_length > kMaxDescriptor ||
        r->input_length != 64 || r->output_length > kMaxReport ||
        r->feature_length > kMaxReport ||
        !((r->vid == 0x054c && r->pid == 0x0ce6) ||
          (r->vid == 0x1532 && (r->pid == 0x1024 || r->pid == 0x1026))))
        return STATUS_INVALID_PARAMETER;
    for (size_t i = 0; i < kMaxFeatures; ++i)
        if (r->feature_lengths[i] > r->feature_length) return STATUS_INVALID_PARAMETER;
    AcquireSRWLockExclusive(&c->state);
    c->config = *r;
    c->status = {};
    c->status.version = 1;
    c->command[0] = 0; c->command[1] = 0;
    neutral_locked(c);
    c->last_input = GetTickCount64();
    ReleaseSRWLockExclusive(&c->state);

    VHF_CONFIG config;
    VHF_CONFIG_INIT(&config, c->target_file, c->config.descriptor_length, c->config.descriptor);
    config.VhfClientContext = c;
    config.VendorID = c->config.vid;
    config.ProductID = c->config.pid;
    config.VersionNumber = c->config.revision;
    // Keep PnP identity consistent with the HID attributes games query.
    static wchar_t sony_ids[] = L"HID\\VID_054C&PID_0CE6\0";
    static wchar_t razer_usb_ids[] = L"HID\\VID_1532&PID_1024\0";
    static wchar_t razer_wireless_ids[] = L"HID\\VID_1532&PID_1026\0";
    if (c->config.vid == 0x054c) {
        config.HardwareIDs = sony_ids; config.HardwareIDsLength = sizeof(sony_ids);
    } else if (c->config.pid == 0x1024) {
        config.HardwareIDs = razer_usb_ids; config.HardwareIDsLength = sizeof(razer_usb_ids);
    } else {
        config.HardwareIDs = razer_wireless_ids; config.HardwareIDsLength = sizeof(razer_wireless_ids);
    }
    config.EvtVhfAsyncOperationGetFeature = get_feature;
    config.EvtVhfAsyncOperationSetFeature = set_feature;
    config.EvtVhfAsyncOperationGetInputReport = get_input;
    config.EvtVhfAsyncOperationWriteReport = write_output;
    static wchar_t instance[] = L"RaijuIdentityProbe";
    config.InstanceID = instance;
    config.InstanceIDLength = sizeof(instance);
    NTSTATUS status = VhfCreate(&config, &c->vhf);
    if (!NT_SUCCESS(status)) return status;
    c->owner = owner;
    status = VhfStart(c->vhf);
    if (!NT_SUCCESS(status)) { destroy_locked(c); return status; }
    AcquireSRWLockExclusive(&c->state);
    c->status.active = 1;
    ReleaseSRWLockExclusive(&c->state);
    // No synthetic button input: the client sends unmodified physical reports.
    return STATUS_SUCCESS;
}

void get_feature(PVOID opaque, VHFOPERATIONHANDLE operation, PVOID, PHID_XFER_PACKET p) {
    auto* c = static_cast<DeviceContext*>(opaque);
    NTSTATUS status = STATUS_NOT_SUPPORTED;
    AcquireSRWLockExclusive(&c->state);
    ++c->status.get_feature;
    c->status.last_feature_id = p->reportId;
    for (size_t i = 0; i < kMaxFeatures; ++i) {
        const auto length = c->config.feature_lengths[i];
        if (length && c->config.features[i][0] == p->reportId) {
            if (!p->reportBuffer || p->reportBufferLen < length) status = STATUS_BUFFER_TOO_SMALL;
            else {
                std::memset(p->reportBuffer, 0, p->reportBufferLen);
                std::memcpy(p->reportBuffer, c->config.features[i], length);
                if (p->reportId == 0x81 && length >= 8 && c->command[0] != 1) {
                    std::memset(p->reportBuffer, 0, p->reportBufferLen);
                    p->reportBuffer[0] = 0x81;
                    if (c->command[0] == 3) {
                        p->reportBuffer[1] = 1; p->reportBuffer[4] = 1;
                    } else if (c->command[0] == 4) {
                        // Virtual-device fixture values, not physical measurements.
                        p->reportBuffer[4] = 0xd8; p->reportBuffer[5] = 0x0e;
                        p->reportBuffer[6] = 0xb6; p->reportBuffer[7] = 0x08;
                    } else p->reportBuffer[1] = 1;
                }
                status = STATUS_SUCCESS;
            }
            break;
        }
    }
    if (status == STATUS_NOT_SUPPORTED) ++c->status.unsupported_feature;
    ReleaseSRWLockExclusive(&c->state);
    (void)VhfAsyncOperationComplete(operation, status);
}

void set_feature(PVOID opaque, VHFOPERATIONHANDLE operation, PVOID, PHID_XFER_PACKET p) {
    auto* c = static_cast<DeviceContext*>(opaque);
    AcquireSRWLockExclusive(&c->state);
    ++c->status.set_feature;
    c->status.last_feature_id = p->reportId;
    c->status.last_set_feature = p->reportId;
    NTSTATUS status = STATUS_NOT_SUPPORTED;
    const bool compatible = c->config.feature_lengths[0] == 41 && c->config.features[0][0] == 5;
    if (compatible && p->reportBuffer && p->reportBufferLen > 0 &&
        p->reportBufferLen <= c->config.feature_length && p->reportBuffer[0] == p->reportId) {
        status = STATUS_SUCCESS;
        if (p->reportBufferLen >= 3) {
            c->status.last_set_feature |= (static_cast<uint32_t>(p->reportBuffer[1]) << 8) |
                                         (static_cast<uint32_t>(p->reportBuffer[2]) << 16);
            if (p->reportId == 0x80) {
                c->command[0] = p->reportBuffer[1];
                c->command[1] = p->reportBuffer[2];
            }
        }
    }
    ReleaseSRWLockExclusive(&c->state);
    // This modifies only the virtual session. No write reaches the Raiju.
    (void)VhfAsyncOperationComplete(operation, status);
}

void get_input(PVOID opaque, VHFOPERATIONHANDLE operation, PVOID, PHID_XFER_PACKET p) {
    auto* c = static_cast<DeviceContext*>(opaque);
    NTSTATUS status = STATUS_INVALID_PARAMETER;
    AcquireSRWLockExclusive(&c->state);
    ++c->status.get_input;
    if (p->reportId == 1 && p->reportBuffer && p->reportBufferLen >= c->config.input_length) {
        std::memset(p->reportBuffer, 0, p->reportBufferLen);
        std::memcpy(p->reportBuffer, c->input, c->config.input_length);
        status = STATUS_SUCCESS;
    }
    ReleaseSRWLockExclusive(&c->state);
    (void)VhfAsyncOperationComplete(operation, status);
}

void write_output(PVOID opaque, VHFOPERATIONHANDLE operation, PVOID, PHID_XFER_PACKET p) {
    auto* c = static_cast<DeviceContext*>(opaque);
    AcquireSRWLockExclusive(&c->state);
    ++c->status.output;
    const bool valid = p->reportId == 2 && p->reportBuffer && p->reportBufferLen == c->config.output_length;
    ReleaseSRWLockExclusive(&c->state);
    // Output is acknowledged but rumble is not forwarded in the identity test.
    (void)VhfAsyncOperationComplete(operation, valid ? STATUS_SUCCESS : STATUS_INVALID_PARAMETER);
}

void device_control(WDFQUEUE queue, WDFREQUEST request, size_t output_length,
                    size_t input_length, ULONG code) {
    auto* c = context_of(WdfIoQueueGetDevice(queue));
    NTSTATUS status = STATUS_INVALID_DEVICE_REQUEST;
    ULONG_PTR information = 0;
    AcquireSRWLockExclusive(&c->lifetime);
    WDFFILEOBJECT owner = WdfRequestGetFileObject(request);
    if (code == kIoctlStatus && !input_length && output_length >= sizeof(DriverStatus)) {
        void* output = nullptr;
        status = WdfRequestRetrieveOutputBuffer(request, sizeof(DriverStatus), &output, nullptr);
        if (NT_SUCCESS(status)) {
            AcquireSRWLockShared(&c->state);
            std::memcpy(output, &c->status, sizeof(c->status));
            ReleaseSRWLockShared(&c->state);
            information = sizeof(DriverStatus);
        }
    } else if (code == kIoctlCreate && input_length == sizeof(CreateRequest) && !output_length) {
        CreateRequest* input = nullptr;
        status = WdfRequestRetrieveInputBuffer(request, sizeof(CreateRequest), reinterpret_cast<void**>(&input), nullptr);
        if (NT_SUCCESS(status)) status = create_locked(c, owner, input);
    } else if (code == kIoctlInput && input_length == 64 && !output_length) {
        uint8_t* input = nullptr;
        status = WdfRequestRetrieveInputBuffer(request, 64, reinterpret_cast<void**>(&input), nullptr);
        if (NT_SUCCESS(status)) {
            if (!c->vhf || c->owner != owner) status = STATUS_ACCESS_DENIED;
            else if (input[0] != 1) status = STATUS_INVALID_PARAMETER;
            else {
                AcquireSRWLockExclusive(&c->state);
                std::memcpy(c->input, input, 64);
                c->last_input = GetTickCount64();
                ReleaseSRWLockExclusive(&c->state);
                status = submit_locked(c, input);
            }
        }
    } else if (code == kIoctlDestroy && !input_length && !output_length) {
        if (c->vhf && c->owner != owner) status = STATUS_ACCESS_DENIED;
        else { destroy_locked(c); status = STATUS_SUCCESS; }
    }
    ReleaseSRWLockExclusive(&c->lifetime);
    WdfRequestCompleteWithInformation(request, status, information);
}

void file_cleanup(WDFFILEOBJECT file) {
    auto* c = context_of(WdfFileObjectGetDevice(file));
    AcquireSRWLockExclusive(&c->lifetime);
    if (c->owner == file) destroy_locked(c);
    ReleaseSRWLockExclusive(&c->lifetime);
}

void watchdog(WDFTIMER timer) {
    auto* c = context_of(reinterpret_cast<WDFDEVICE>(WdfTimerGetParentObject(timer)));
    AcquireSRWLockExclusive(&c->lifetime);
    if (c->vhf) {
        uint8_t neutral[kMaxReport]{};
        bool stale = false;
        AcquireSRWLockExclusive(&c->state);
        if (GetTickCount64() - c->last_input > 250) {
            neutral_locked(c);
            std::memcpy(neutral, c->input, sizeof(neutral));
            ++c->status.watchdog_resets;
            c->last_input = GetTickCount64();
            stale = true;
        }
        ReleaseSRWLockExclusive(&c->state);
        if (stale) (void)submit_locked(c, neutral);
    }
    ReleaseSRWLockExclusive(&c->lifetime);
    WdfTimerStart(timer, WDF_REL_TIMEOUT_IN_MS(50));
}

NTSTATUS d0_entry(WDFDEVICE device, WDF_POWER_DEVICE_STATE) {
    auto* c = context_of(device);
    AcquireSRWLockExclusive(&c->lifetime);
    WDF_IO_TARGET_OPEN_PARAMS parameters;
    WDF_IO_TARGET_OPEN_PARAMS_INIT_OPEN_BY_FILE(&parameters, nullptr);
    NTSTATUS status = WdfIoTargetOpen(c->target, &parameters);
    if (NT_SUCCESS(status)) {
        c->target_file = WdfIoTargetWdmGetTargetFileHandle(c->target);
        c->powered = c->target_file != nullptr;
        if (!c->powered) { WdfIoTargetClose(c->target); status = STATUS_DEVICE_NOT_READY; }
    }
    ReleaseSRWLockExclusive(&c->lifetime);
    return status;
}

NTSTATUS d0_exit(WDFDEVICE device, WDF_POWER_DEVICE_STATE) {
    auto* c = context_of(device);
    AcquireSRWLockExclusive(&c->lifetime);
    c->powered = false;
    destroy_locked(c);
    c->target_file = nullptr;
    WdfIoTargetClose(c->target);
    ReleaseSRWLockExclusive(&c->lifetime);
    return STATUS_SUCCESS;
}

NTSTATUS device_add(WDFDRIVER, PWDFDEVICE_INIT init) {
    WDF_FILEOBJECT_CONFIG files;
    WDF_FILEOBJECT_CONFIG_INIT(&files, WDF_NO_EVENT_CALLBACK, WDF_NO_EVENT_CALLBACK, file_cleanup);
    WdfDeviceInitSetFileObjectConfig(init, &files, WDF_NO_OBJECT_ATTRIBUTES);
    WDF_PNPPOWER_EVENT_CALLBACKS power;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&power);
    power.EvtDeviceD0Entry = d0_entry; power.EvtDeviceD0Exit = d0_exit;
    WdfDeviceInitSetPnpPowerEventCallbacks(init, &power);
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, DeviceContext);
    attributes.ExecutionLevel = WdfExecutionLevelPassive;
    attributes.SynchronizationScope = WdfSynchronizationScopeNone;
    WDFDEVICE device;
    NTSTATUS status = WdfDeviceCreate(&init, &attributes, &device);
    if (!NT_SUCCESS(status)) return status;
    auto* c = context_of(device);
    InitializeSRWLock(&c->lifetime); InitializeSRWLock(&c->state);
    c->status.version = 1;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = device;
    status = WdfIoTargetCreate(device, &attributes, &c->target);
    if (!NT_SUCCESS(status)) return status;
    status = WdfDeviceCreateDeviceInterface(device, &kControlInterface, nullptr);
    if (!NT_SUCCESS(status)) return status;
    WDF_IO_QUEUE_CONFIG queue;
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&queue, WdfIoQueueDispatchSequential);
    queue.EvtIoDeviceControl = device_control;
    status = WdfIoQueueCreate(device, &queue, WDF_NO_OBJECT_ATTRIBUTES, nullptr);
    if (!NT_SUCCESS(status)) return status;
    WDF_TIMER_CONFIG timer;
    WDF_TIMER_CONFIG_INIT(&timer, watchdog);
    timer.AutomaticSerialization = FALSE;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = device;
    attributes.ExecutionLevel = WdfExecutionLevelPassive;
    WDFTIMER handle;
    status = WdfTimerCreate(&timer, &attributes, &handle);
    if (NT_SUCCESS(status)) WdfTimerStart(handle, WDF_REL_TIMEOUT_IN_MS(50));
    return status;
}

extern "C" NTSTATUS DriverEntry(PDRIVER_OBJECT object, PUNICODE_STRING path) {
    WDF_DRIVER_CONFIG config;
    WDF_DRIVER_CONFIG_INIT(&config, device_add);
    return WdfDriverCreate(object, path, WDF_NO_OBJECT_ATTRIBUTES, &config, WDF_NO_HANDLE);
}
