use detail::{align_down, mut_offset};
use reg_context::InitFn;
use stack::Stack;

#[cfg(not(nightly))]
#[link(name = "asm", kind = "static")]
extern "C" {
    pub fn bootstrap_green_task();
    pub fn prefetch_asm(data: *const usize);
    pub fn swap_registers(out_regs: *mut Registers, in_regs: *const Registers);
}

#[allow(dead_code)]
#[inline]
pub fn prefetch(data: *const usize) {
    unsafe {
        prefetch_asm(data);
    }
}

#[cfg(nightly)]
mod asm {
    use super::Registers;
    /// prefetch data
    #[inline]
    pub unsafe fn prefetch_asm(data: *const usize) {
        asm!("prefetcht1 $0"
             : // no output
             : "m"(*data)
             :
             : "volatile");
    }

    #[inline(never)]
    #[naked]
    pub unsafe extern "C" fn bootstrap_green_task() {
        asm!("
            mov %r12, %rcx     // setup the function arg
            mov %r13, %rdx     // setup the function arg
            mov %r14, 8(%rsp)  // this is the new return adrress
        "
        : // no output
        : // no input
        : "memory"
        : "volatile");
    }

    #[inline(never)]
    #[naked]
    pub unsafe extern "C" fn swap_registers(out_regs: *mut Registers, in_regs: *const Registers) {
        // The first argument is in %rcx, and the second one is in %rdx

        // Save registers
        asm!("
            mov %rbx, (0*8)(%rcx)
            mov %rsp, (1*8)(%rcx)
            mov %rbp, (2*8)(%rcx)
            mov %r12, (4*8)(%rcx)
            mov %r13, (5*8)(%rcx)
            mov %r14, (6*8)(%rcx)
            mov %r15, (7*8)(%rcx)
            mov %rdi, (9*8)(%rcx)
            mov %rsi, (10*8)(%rcx)

            // mov %rcx, %r10
            // and $$0xf0, %r10b
    
            // Save non-volatile XMM registers:
            movapd %xmm6, (16*8)(%rcx)
            movapd %xmm7, (18*8)(%rcx)
    
            /* load NT_TIB */
            movq  %gs:(0x30), %r10
            /* save current stack base */
            movq  0x08(%r10), %rax
            mov  %rax, (11*8)(%rcx)
            /* save current stack limit */
            movq  0x10(%r10), %rax
             mov  %rax, (12*8)(%rcx)
            /* save current deallocation stack */
            movq  0x1478(%r10), %rax
            mov  %rax, (13*8)(%rcx)
            /* save fiber local storage */
            // movq  0x18(%r10), %rax
            // mov  %rax, (14*8)(%rcx)
    
            mov %rcx, (3*8)(%rcx)
    
            mov (0*8)(%rdx), %rbx
            mov (1*8)(%rdx), %rsp
            mov (2*8)(%rdx), %rbp
            mov (4*8)(%rdx), %r12
            mov (5*8)(%rdx), %r13
            mov (6*8)(%rdx), %r14
            mov (7*8)(%rdx), %r15
            mov (9*8)(%rdx), %rdi
            mov (10*8)(%rdx), %rsi
    
            // Restore non-volatile XMM registers:
            movapd (16*8)(%rdx), %xmm6
            movapd (18*8)(%rdx), %xmm7
    
            /* load NT_TIB */
            movq  %gs:(0x30), %r10
            /* restore fiber local storage */
            // mov (14*8)(%rdx), %rax
            // movq  %rax, 0x18(%r10)
            /* restore deallocation stack */
            mov (13*8)(%rdx), %rax
            movq  %rax, 0x1478(%r10)
            /* restore stack limit */
            mov (12*8)(%rdx), %rax
            movq  %rax, 0x10(%r10)
            /* restore stack base */
            mov  (11*8)(%rdx), %rax
            movq  %rax, 0x8(%r10)
    
            mov (3*8)(%rdx), %rcx
        "
        :
        : "{rcx}"(out_regs), "{rdx}"(in_regs)
        : "memory"
        : "volatile");
    }
}
#[cfg(nightly)]
pub use self::asm::*;

#[cfg_attr(nightly, repr(simd))]
#[cfg_attr(not(nightly), repr(C))]
#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
struct XMM(u32, u32, u32, u32);

impl XMM {
    pub fn new(a: u32, b: u32, c: u32, d: u32) -> Self {
        XMM(a, b, c, d)
    }
}

// windows need to restore xmm6~xmm15, for most cases only use two xmm registers
#[repr(C)]
#[derive(Debug)]
pub struct Registers {
    gpr: [usize; 16],
    #[cfg(nightly)]
    _xmm: [XMM; 2],
    // keep enough for place holder
    #[cfg(not(nightly))]
    _xmm: [XMM; 4],
}

impl Registers {
    pub fn new() -> Registers {
        Registers {
            gpr: [0; 16],
            #[cfg(nightly)]
            _xmm: [XMM::new(0, 0, 0, 0); 2],
            #[cfg(not(nightly))]
            _xmm: [XMM::new(0, 0, 0, 0); 4],
        }
    }

    #[inline]
    pub fn prefetch(&self) {
        unsafe {
           prefetch_asm(self as *const _ as *const usize);
           prefetch_asm(self.gpr[1] as *const usize);
        }
    }
}

pub fn initialize_call_frame(
    regs: &mut Registers,
    fptr: InitFn,
    arg: usize,
    arg2: *mut usize,
    stack: &Stack,
) {
    // Redefinitions from rt/arch/x86_64/regs.h
    const RUSTRT_RSP: usize = 1;
    const RUSTRT_RBP: usize = 2;
    const RUSTRT_R12: usize = 4;
    const RUSTRT_R13: usize = 5;
    const RUSTRT_R14: usize = 6;
    const RUSTRT_STACK_BASE: usize = 11;
    const RUSTRT_STACK_LIMIT: usize = 12;
    const RUSTRT_STACK_DEALLOC: usize = 13;

    let sp = align_down(stack.end());

    // These registers are frobbed by rust_bootstrap_green_task into the right
    // location so we can invoke the "real init function", `fptr`.
    regs.gpr[RUSTRT_R12] = arg;
    regs.gpr[RUSTRT_R13] = arg2 as usize;
    regs.gpr[RUSTRT_R14] = fptr as usize;

    // These registers are picked up by the regular context switch paths. These
    // will put us in "mostly the right context" except for frobbing all the
    // arguments to the right place. We have the small trampoline code inside of
    // rust_bootstrap_green_task to do that.
    regs.gpr[RUSTRT_RSP] = mut_offset(sp, -4) as usize;

    // Last base pointer on the stack should be 0
    regs.gpr[RUSTRT_RBP] = 0;

    regs.gpr[RUSTRT_STACK_BASE] = stack.end() as usize;
    regs.gpr[RUSTRT_STACK_LIMIT] = stack.begin() as usize;
    regs.gpr[RUSTRT_STACK_DEALLOC] = 0; //mut_offset(sp, -8192) as usize;

    // setup the init stack
    // this is prepared for the swap context
    // different platform/debug has different offset between sp and ret
    unsafe {
        *mut_offset(sp, -4) = bootstrap_green_task as usize;
        *mut_offset(sp, -3) = bootstrap_green_task as usize;
        // leave enough space for RET
        *mut_offset(sp, -2) = 0;
        *mut_offset(sp, -1) = 0;
    }
}
