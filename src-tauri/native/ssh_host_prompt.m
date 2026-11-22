#import <AppKit/AppKit.h>
#include <stdint.h>

@interface FLOESSHHostForm : NSObject
@property(strong) NSButton *verified;
@property(strong) NSButton *approve;
- (void)changed:(id)sender;
@end
@implementation FLOESSHHostForm
- (void)changed:(id)sender {
    (void)sender;
    self.approve.enabled = self.verified.state == NSControlStateValueOn;
}
@end

// Main-thread-only presentation boundary. This function never stores a pin or
// starts a connection. Caller must revalidate the frozen trust revision on return.
int32_t branchdesk_prompt_ssh_host(const char *destination, const char *algorithm,
                                  const char *fingerprint) {
    @autoreleasepool {
        if (![NSThread isMainThread] || NSApp == nil || !destination || !algorithm || !fingerprint) return -1;
        @try {
            NSString *address = [NSString stringWithUTF8String:destination];
            NSString *kind = [NSString stringWithUTF8String:algorithm];
            NSString *digest = [NSString stringWithUTF8String:fingerprint];
            for (NSString *field in @[address ?: @"", kind ?: @"", digest ?: @""]) {
                if (!field.length || field.length > 8192 ||
                    [field rangeOfCharacterFromSet:NSCharacterSet.controlCharacterSet].location != NSNotFound) return -1;
            }
            NSAlert *alert = [NSAlert new];
            alert.alertStyle = NSAlertStyleWarning;
            alert.messageText = @"Verify SSH host key";
            alert.informativeText = @"This host is not trusted yet. Compare the fingerprint with a trusted source from the server administrator before saving it. Discovery alone does not verify the server's identity.";
            NSButton *cancel = [alert addButtonWithTitle:@"Cancel"];
            cancel.keyEquivalent = @"\e";
            NSButton *approve = [alert addButtonWithTitle:@"Trust this host key"];
            approve.keyEquivalent = @"";
            approve.enabled = NO;
            NSView *view = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 540, 202)];
            NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:NSMakeRect(0, 48, 540, 154)];
            scroll.hasVerticalScroller = YES;
            NSTextView *text = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, 520, 154)];
            text.editable = NO; text.selectable = YES; text.richText = NO;
            text.verticallyResizable = YES; text.horizontallyResizable = NO;
            text.autoresizingMask = NSViewWidthSizable;
            text.textContainer.widthTracksTextView = YES;
            text.textContainer.containerSize = NSMakeSize(520, CGFLOAT_MAX);
            text.font = [NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightRegular];
            text.string = [NSString stringWithFormat:@"Destination\n%@\n\nHost key algorithm\n%@\n\nFingerprint\n%@", address, kind, digest];
            text.accessibilityLabel = @"SSH host destination, algorithm and fingerprint";
            scroll.documentView = text;
            [view addSubview:scroll];
            FLOESSHHostForm *form = [FLOESSHHostForm new];
            form.approve = approve;
            form.verified = [NSButton checkboxWithTitle:@"I verified this fingerprint with a trusted source" target:form action:@selector(changed:)];
            form.verified.frame = NSMakeRect(0, 10, 540, 28);
            [view addSubview:form.verified];
            alert.accessoryView = view;
            [alert layout];
            alert.window.defaultButtonCell = cancel.cell;
            // NSAlert's default-button assignment resets its key equivalent.
            // Keep Return safe and handle Escape only while this alert is modal.
            id escapeMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown handler:^NSEvent *(NSEvent *event) {
                if (NSApp.modalWindow == alert.window && event.keyCode == 53 &&
                    !(event.modifierFlags & (NSEventModifierFlagCommand | NSEventModifierFlagControl | NSEventModifierFlagOption | NSEventModifierFlagShift))) {
                    [NSApp abortModal];
                    return nil;
                }
                return event;
            }];
            if (!escapeMonitor) return -1;
            NSModalResponse response;
            @try { response = [alert runModal]; }
            @finally { [NSEvent removeMonitor:escapeMonitor]; }
            return response == NSAlertSecondButtonReturn && form.verified.state == NSControlStateValueOn ? 1 : 0;
        } @catch (NSException *exception) {
            (void)exception;
            return -2;
        }
    }
}
