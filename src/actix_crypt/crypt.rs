#![allow(dead_code)]

use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::ops::{BitAnd, Not};

use aes;

use block_modes::block_padding::Pkcs7;
use block_modes::{BlockMode, BlockModeError, Cbc};

use aes::Aes256;

use block_cipher_trait::generic_array::typenum::Unsigned;
use block_cipher_trait::generic_array::ArrayLength;
use block_cipher_trait::generic_array::GenericArray;
use block_cipher_trait::BlockCipher;

use hex;

use sha2::{Digest, Sha256};

use num_traits::Num;

use lazy_static::lazy_static;

// create an alias for convinience
type Aes256Cbc = Cbc<Aes256, Pkcs7>;

lazy_static! {
    pub static ref AES_KEY: String = std::env::var("AES_KEY").expect("AES_KEY must be set");
    pub static ref BLOB_MAGIC: String = std::env::var("BLOB_MAGIC").expect("BLOB_MAGIC must be set");
}

const MAGIC_SIZE: usize = 0x8;
const RESERVED_SIZE: usize = 0x8;
const INITIAL_VECTOR_SIZE: usize = 0x10;
const HASH_SIZE: usize = 0x20;

pub const HEADER_SIZE: usize = MAGIC_SIZE + RESERVED_SIZE + INITIAL_VECTOR_SIZE + HASH_SIZE;

const BUFFER_SIZE: usize = 65_536;

pub type BlobMagic = [u8; MAGIC_SIZE];
pub type BlobInitialVector = [u8; INITIAL_VECTOR_SIZE];
pub type BlobHash = [u8; HASH_SIZE];

pub struct EncryptedBlob<T: Read + Seek + Sized> {
    accessor: T,
    size: u64,
    cipher: Option<Aes256Cbc>,
    iv: Option<BlobInitialVector>,
}

impl<T: Read + Seek + Sized> EncryptedBlob<T> {
    pub fn from(accessor: T) -> std::io::Result<Self> {
        let mut accessor = accessor;

        // Compute stream size
        accessor.seek(SeekFrom::Start(0))?;
        let size = accessor.seek(SeekFrom::End(0))?;

        let mut res = EncryptedBlob {
            accessor,
            size,
            cipher: None,
            iv: None,
        };
        let key = hex::decode(AES_KEY.as_str()).unwrap();
        let iv = res.initial_vector()?;
        let cipher = Aes256Cbc::new_var(&key, &iv).unwrap();

        // Reset stream position and setup the cipher
        res.accessor.seek(SeekFrom::Start(0))?;
        res.iv = Some(iv);
        res.cipher = Some(cipher);
        Ok(res)
    }

    fn reset_cipher(&mut self) {
        if let Some(iv) = self.iv {
            let key = hex::decode(AES_KEY.as_str()).unwrap();
            self.cipher = Some(Aes256Cbc::new_var(&key, &iv).unwrap());
        } else {
            panic!();
        }
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn is_header_magic_valid(&mut self) -> bool {
        let magic_opt = self.magic();

        if let Ok(magic) = magic_opt {
            return magic == BLOB_MAGIC.as_bytes();
        }
        false
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn is_content_valid(&mut self) -> bool {
        if self
            .accessor
            .seek(SeekFrom::Start(HEADER_SIZE as u64))
            .is_ok()
        {
            let mut internal_buffer = Vec::new();
            internal_buffer.resize(BUFFER_SIZE, 0);

            let mut index = self.size as usize - HEADER_SIZE as usize;

            let mut hasher = Sha256::new();

            while index != 0 {
                let slice = if index < BUFFER_SIZE {
                    &mut internal_buffer[..index]
                } else {
                    &mut internal_buffer[..BUFFER_SIZE]
                };

                let res = self.read(slice);
                if res.is_err() {
                    return false;
                }

                let read_len = res.unwrap();

                index -= slice.len();

                hasher.input(&slice[..read_len]);
            }

            let computed_hash = hasher.result();
            if let Ok(hash) = self.hash() {
                return computed_hash == GenericArray::from(hash);
            }
        }

        false
    }

    pub fn magic(&mut self) -> std::io::Result<BlobMagic> {
        let mut result = [0; MAGIC_SIZE];

        self.accessor.seek(SeekFrom::Start(0))?;
        self.accessor.read_exact(&mut result)?;

        Ok(result)
    }

    pub fn initial_vector(&mut self) -> std::io::Result<BlobInitialVector> {
        let mut result = [0; INITIAL_VECTOR_SIZE];

        self.accessor.seek(SeekFrom::Start((MAGIC_SIZE + RESERVED_SIZE) as u64))?;
        self.accessor.read_exact(&mut result)?;

        Ok(result)
    }

    pub fn hash(&mut self) -> std::io::Result<BlobHash> {
        let mut result = [0; HASH_SIZE];

        self.accessor
            .seek(SeekFrom::Start((MAGIC_SIZE + RESERVED_SIZE + INITIAL_VECTOR_SIZE) as u64))?;
        self.accessor.read_exact(&mut result)?;

        Ok(result)
    }

    pub fn encrypted_data(&mut self) -> std::io::Result<Vec<u8>> {
        let mut result = Vec::new();

        self.accessor.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        self.accessor.read_to_end(&mut result)?;

        Ok(result)
    }

    pub fn decrypted_data(&mut self) -> Result<Vec<u8>, BlockModeError> {
        let key = hex::decode(AES_KEY.as_str()).unwrap();

        if let Some(iv) = self.iv {
            let cipher = Aes256Cbc::new_var(&key, &iv).unwrap();
            let encrypted_data = self.encrypted_data().unwrap();
            cipher.decrypt_vec(&encrypted_data)
        } else {
            panic!();
        }
    }

    pub fn get_unpadded_size(&mut self) -> std::io::Result<u64> {
        let padded_size = self.get_padded_size();
        let block_size = <Aes256 as BlockCipher>::BlockSize::to_usize();

        let last_block_index = ((padded_size / block_size as u64) - 1) * block_size as u64;

        // Reset accessor
        self.accessor.seek(SeekFrom::Start(0))?;

        // Use our seek to get the right IV
        self.seek(SeekFrom::Start(last_block_index))?;
        let mut data = Vec::new();
        data.resize(block_size, 0);

        // Manually read and decrypt the last block to avoid unpadding
        self.accessor.read_exact(&mut data)?;

        let mut cipher = self.cipher.take().expect("Cipher not availaible!");
        cipher.decrypt_blocks(to_blocks(&mut data));

        self.cipher = Some(cipher);

        Ok(padded_size - u64::from(data[data.len() - 1]))
    }

    pub fn get_padded_size(&self) -> u64 {
        self.size - HEADER_SIZE as u64
    }

    pub fn into_inner(self) -> T {
        self.accessor
    }
}

impl<T: Read + Seek + Sized> Seek for EncryptedBlob<T> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let mut current_pos = self.accessor.seek(SeekFrom::Current(0))? as i64;

        // Fix position if not in the right range
        if current_pos == 0 {
            // Get back to the begining of the AES blob
            current_pos = self.accessor.seek(SeekFrom::Start(HEADER_SIZE as u64))? as i64;
        }

        let (need_skip_header, mut pos_from_start) = match pos {
            SeekFrom::Start(pos) => (true, pos as i64),
            // We don't want to be past the file
            SeekFrom::End(_) => (false, self.size as i64),
            SeekFrom::Current(pos) => (false, current_pos + pos as i64),
        };

        // If it's unaligned to a block size, we error out!
        // TODO: support unaligned position?
        if current_pos % 0x10 != 0 || pos_from_start % 0x10 != 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        }

        if need_skip_header {
            pos_from_start += HEADER_SIZE as i64;
        }

        if pos_from_start != current_pos {
            let mut diff = if pos_from_start > current_pos {
                // We need to go forward to update the internal IV
                (pos_from_start - current_pos) as usize
            } else {
                // We need to reset the cipher instance
                self.reset_cipher();
                pos_from_start as usize
            };

            let mut internal_buffer = Vec::new();
            internal_buffer.resize(BUFFER_SIZE, 0);

            while diff != 0 {
                let slice = if diff < BUFFER_SIZE {
                    &mut internal_buffer[..diff]
                } else {
                    &mut internal_buffer[..BUFFER_SIZE]
                };

                self.read_exact(slice)?;
                diff -= slice.len();
            }
        }

        let new_position = self.accessor.seek(SeekFrom::Current(0))? as i64;
        assert!(new_position == pos_from_start, "Position isn't correct!");

        let pos_from_start = match pos {
            SeekFrom::End(_) => pos_from_start,
            _ => {
                if pos_from_start == current_pos {
                    current_pos
                } else {
                    pos_from_start - HEADER_SIZE as i64
                }
            }
        };

        Ok(pos_from_start as u64)
    }
}

pub(crate) fn to_blocks<N>(data: &mut [u8]) -> &mut [GenericArray<u8, N>]
where
    N: ArrayLength<u8>,
{
    let n = N::to_usize();
    debug_assert!(data.len() % n == 0);
    unsafe {
        std::slice::from_raw_parts_mut(data.as_ptr() as *mut GenericArray<u8, N>, data.len() / n)
    }
}

pub fn align_down<T: Num + Not<Output = T> + BitAnd<Output = T> + Copy>(addr: T, align: T) -> T {
    addr & !(align - T::one())
}

pub fn align_up<T: Num + Not<Output = T> + BitAnd<Output = T> + Copy>(addr: T, align: T) -> T {
    align_down(addr + (align - T::one()), align)
}

impl<T: Read + Seek + Sized> Read for EncryptedBlob<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Cipher is gone, we are done
        if self.cipher.is_none() {
            return Ok(0);
        }
        let mut cipher = self.cipher.take().unwrap();

        let mut internal_buffer = Vec::new();
        internal_buffer.resize(align_up(buf.len(), 0x10), 0);

        let mut read_len = self.accessor.read(&mut internal_buffer)?;
        if read_len != internal_buffer.len() {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
        }

        let current_pos = self.accessor.seek(SeekFrom::Current(0))? - HEADER_SIZE as u64;

        if current_pos == self.get_padded_size() {
            let res = cipher.decrypt(&mut internal_buffer);
            if res.is_err() {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
            }

            let slice = res.unwrap();
            read_len = slice.len();
        } else {
            cipher.decrypt_blocks(to_blocks(&mut internal_buffer));
            self.cipher = Some(cipher);
            read_len = buf.len();
        }

        buf.copy_from_slice(&internal_buffer[..buf.len()]);
        Ok(read_len)
    }
}
